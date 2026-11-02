//! Child authoring uses local configuration plus the node's ordinary validation gates.
use crate::{
    keys,
    lifecycle::{
        config::encoded,
        files::{self, Layout},
        reconcile_hosted,
    },
    rpc::LocalRpc,
    topology::Topology,
    transaction::{self, ActionPlan},
};
use anyhow::{Context, Result, ensure};
use lattice_sdk::{primitives::*, submission::HttpTransactionSubmitter};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio_util::sync::CancellationToken;

pub struct Create {
    pub path: String,
    pub spec: Option<PathBuf>,
    pub policy_modules: Vec<PathBuf>,
    pub block_time: u64,
    pub reward: u64,
    pub premine: u64,
    pub premine_to: Option<String>,
    pub genesis_transactions: Option<PathBuf>,
}
fn hosted(topology: &Topology, path: &str) -> Result<Topology> {
    ensure!(
        !topology.hosted_paths().contains(&path.to_owned()),
        "child is already hosted"
    );
    let mut next = topology.clone();
    next.hosted_chains.get_or_insert_default().push(path.into());
    next.validate()?;
    Ok(next)
}
async fn save(layout: &Layout, topology: &Topology) -> Result<Value> {
    files::stage_topology(layout, topology)?;
    let applied = reconcile_hosted(layout, topology).await.context(
        "child configuration remains pending; artifacts preserved; retry child apply or up",
    )?;
    files::finish_topology(layout, topology)?;
    Ok(json!({"configured":true,"appliedToRunningNode":applied}))
}
pub async fn host(root: &Path, path: &str) -> Result<Value> {
    let layout = Layout::open(root)?;
    let _lock = layout.lock()?;
    files::ensure_no_pending(&layout)?;
    let topology = hosted(&Topology::load(&layout.root)?, path)?;
    save(&layout, &topology).await
}
pub async fn apply(root: &Path) -> Result<Value> {
    let layout = Layout::open(root)?;
    let _lock = layout.lock()?;
    let topology = files::desired_topology(&layout)?;
    let applied = reconcile_hosted(&layout, &topology)
        .await
        .context("pending configuration retained; retry child apply or up")?;
    files::finish_topology(&layout, &topology)?;
    Ok(json!({"appliedToRunningNode":applied}))
}
pub async fn create(root: &Path, options: Create) -> Result<Value> {
    let layout = Layout::open(root)?;
    let _lock = layout.lock()?;
    files::ensure_no_pending(&layout)?;
    let topology = hosted(&Topology::load(&layout.root)?, &options.path)?;
    let spec: ChainSpec = match options.spec {
        Some(path) => serde_json::from_slice(&files::read(&path, 4 * 1024 * 1024)?)?,
        None => ChainSpec {
            max_number_of_transactions_per_block: 1000,
            max_state_growth: 1_000_000,
            max_block_size: 1_000_000,
            premine: options.premine,
            target_block_time: options.block_time,
            initial_reward: options.reward,
            halving_interval: 1_000_000,
            half_life: 100,
            wasm_policies: vec![],
        },
    };
    ensure!(spec.is_valid(), "invalid chain spec");
    ensure!(
        (spec.premine_amount() > 0) == options.premine_to.is_some(),
        "a positive premine requires premine-to; a zero premine forbids it"
    );
    let mut modules = BTreeMap::new();
    for file in options.policy_modules {
        let bytes = files::read(&file, 4 * 1024 * 1024)?;
        let cid = content_id(&WasmPolicyModule {
            bytes: bytes.clone(),
        })?;
        let policies: Vec<_> = spec
            .wasm_policies
            .iter()
            .filter(|p| p.module_cid == cid)
            .collect();
        ensure!(
            !policies.is_empty(),
            "provided policy module is not named by spec"
        );
        for policy in policies {
            lattice_wasm::WasmPolicy::new(&bytes, policy.abi_version, &policy.entrypoint)?;
        }
        modules.insert(cid, bytes);
    }
    ensure!(
        spec.wasm_policies
            .iter()
            .all(|p| modules.contains_key(&p.module_cid)),
        "spec names a policy module that was not supplied"
    );
    let genesis = options
        .genesis_transactions
        .map(|path| files::read(&path, 4 * 1024 * 1024))
        .transpose()?;
    let transactions = match &genesis {
        None => vec![],
        Some(bytes) => serde_json::from_slice::<Vec<Value>>(bytes)?
            .into_iter()
            .map(|v| {
                lattice_node::runtime::transaction::decode(
                    v.get("transaction")
                        .cloned()
                        .context("genesis envelope requires transaction")?,
                )
                .map_err(anyhow::Error::msg)
            })
            .collect::<Result<Vec<_>>>()?,
    };
    let path: Vec<String> = options.path.split('/').map(str::to_owned).collect();
    lattice_node::runtime::hosted_config::check_premine(
        &spec,
        options.premine_to.as_deref(),
        &transactions,
        &path,
        &modules.values().cloned().collect::<Vec<_>>(),
        i64::try_from(SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis())?,
    )?;
    let directory = layout.ensure("specs")?;
    let name = encoded(&options.path);
    let spec_file = directory.join(format!("{name}.json"));
    let policy_directory = directory.join(format!("{name}.policies"));
    let mut writes = vec![];
    if let Some(to) = options.premine_to {
        writes.push((
            directory.join(format!("{name}.premine-to")),
            to.into_bytes(),
        ));
    }
    if let Some(genesis) = genesis {
        writes.push((directory.join(format!("{name}.genesis.json")), genesis));
    }
    for (cid, bytes) in modules {
        writes.push((policy_directory.join(format!("{cid}.wasm")), bytes));
    }
    writes.push((spec_file, serde_json::to_vec_pretty(&spec)?));
    ensure!(
        writes
            .iter()
            .all(|(path, _)| !path.exists() && !path.is_symlink())
            && !policy_directory.exists(),
        "child artifacts already exist; refusing overwrite"
    );
    let mut created = vec![];
    let result = (|| -> Result<()> {
        if writes
            .iter()
            .any(|(path, _)| path.parent() == Some(policy_directory.as_path()))
        {
            std::fs::create_dir(&policy_directory)?;
        }
        for (path, bytes) in &writes {
            files::write(path, bytes, true)?;
            created.push(path.clone());
        }
        // Durable intent precedes any RPC that could change the live host.
        files::stage_topology(&layout, &topology)?;
        Ok(())
    })();
    if let Err(error) = result {
        // A durable-intent write can fail after rename (e.g. directory fsync).
        // If intent exists, preserve its referenced artifacts even then.
        if files::pending_path(&layout).exists() {
            return Err(error.context(
                "child configuration intent exists; artifacts preserved; retry child apply or up",
            ));
        }
        for path in created.into_iter().rev() {
            let _ = std::fs::remove_file(path);
        }
        let _ = std::fs::remove_dir(&policy_directory);
        return Err(error);
    }
    let applied = reconcile_hosted(&layout, &topology).await.context(
        "child configuration remains pending; artifacts preserved; retry child apply or up",
    )?;
    files::finish_topology(&layout, &topology)?;
    Ok(json!({"created":options.path,"appliedToRunningNode":applied}))
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Built {
    #[serde(rename = "blockCID")]
    block_cid: String,
    parent_state: String,
    previous: Option<String>,
    submit: Value,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ReadCommitment {
    parent_state: String,
    value: Option<String>,
}
#[derive(Debug, PartialEq)]
enum Step {
    Done,
    Wait,
    Read,
    Commit,
    Build,
}
fn step(
    pushed: Option<&str>,
    read: Option<&ReadCommitment>,
    built: Option<&Built>,
    accepted: bool,
) -> Result<Step> {
    match pushed {
        Some("tree") => return Ok(Step::Done),
        Some("pending" | "awaiting") => return Ok(Step::Wait),
        None | Some("dropped") => {}
        _ => anyhow::bail!("unknown child push status"),
    }
    let Some(read) = read else {
        return Ok(Step::Read);
    };
    if let Some(built) = built {
        if read.parent_state == built.parent_state {
            return Ok(if accepted { Step::Wait } else { Step::Commit });
        }
        if read.value.as_deref() == Some(&format!("{}@{}", built.block_cid, built.parent_state)) {
            return Ok(Step::Wait);
        }
    }
    Ok(Step::Build)
}
#[derive(Serialize, Deserialize)]
struct Prepared {
    body: TransactionBody,
    signatures: BTreeMap<String, String>,
}
#[derive(Serialize, Deserialize)]
struct Journal {
    chain: String,
    signer: String,
    built: Option<Built>,
    accepted: bool,
    prepared: Option<Prepared>,
    done: bool,
}
pub struct Commit {
    pub path: String,
    pub key: PathBuf,
    pub fee: Option<u64>,
    pub parent_rpc: Option<String>,
    pub parent_cookie: Option<PathBuf>,
    pub poll_ms: u64,
    pub timeout_seconds: Option<u64>,
}
fn emit(value: Value) {
    println!("{value}");
}
pub async fn commit(root: &Path, options: Commit) -> Result<()> {
    use fs2::FileExt;
    use std::os::unix::fs::OpenOptionsExt;
    let layout = Layout::open(root)?;
    let topology = Topology::load(&layout.root)?;
    ensure!(
        topology
            .hosted_chains
            .as_ref()
            .is_some_and(|paths| paths.contains(&options.path)),
        "child must be hosted"
    );
    ensure!(
        (10..=60000).contains(&options.poll_ms),
        "poll-ms must be between 10 and 60000"
    );
    let path: Vec<_> = options.path.split('/').map(str::to_owned).collect();
    let directory = path.last().context("missing child directory")?.clone();
    let signer = keys::load(&options.key)?;
    let base = format!("http://127.0.0.1:{}", topology.rpc);
    let author = LocalRpc::new(&base, layout.cookie(), &options.path)?;
    ensure!(
        options.parent_rpc.is_some() == options.parent_cookie.is_some(),
        "parent-rpc and parent-cookie must be provided together"
    );
    let parent = LocalRpc::new(
        options.parent_rpc.as_deref().unwrap_or(&base),
        options
            .parent_cookie
            .clone()
            .unwrap_or_else(|| layout.cookie()),
        &path[..path.len() - 1].join("/"),
    )?;
    let run = layout.ensure("run")?;
    let name = encoded(&options.path);
    let lock = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(run.join(format!("child-{name}.lock")))?;
    lock.try_lock_exclusive()
        .context("child commit already running")?;
    let journal_path = run.join(format!("child-{name}.json"));
    let mut journal = if journal_path.exists() {
        serde_json::from_slice::<Journal>(&files::read(&journal_path, 4 * 1024 * 1024)?)?
    } else {
        Journal {
            chain: options.path.clone(),
            signer: signer.address().into(),
            built: None,
            accepted: false,
            prepared: None,
            done: false,
        }
    };
    ensure!(
        journal.chain == options.path && journal.signer == signer.address(),
        "journal belongs to another chain or signer"
    );
    // A completed invocation begins a new block; interrupted invocations resume exactly.
    if journal.done {
        journal = Journal {
            chain: options.path.clone(),
            signer: signer.address().into(),
            built: None,
            accepted: false,
            prepared: None,
            done: false,
        };
    }
    let cancel = CancellationToken::new();
    let signal = cancel.clone();
    let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    let mut interrupt = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt())?;
    let signals = tokio::spawn(async move {
        tokio::select! {_=term.recv()=>{},_=interrupt.recv()=>{}};
        signal.cancel();
    });
    let started = std::time::Instant::now();
    let operation = async {
        loop {
            if options
                .timeout_seconds
                .is_some_and(|seconds| started.elapsed() >= Duration::from_secs(seconds))
            {
                anyhow::bail!("child commit timed out; journal retained for resume")
            }
            let iteration = async {
                let mut pushed = None;
                if let Some(built) = &journal.built {
                    let answer = author
                        .request(
                            reqwest::Method::POST,
                            "/child-blocks",
                            Some(built.submit.clone()),
                        )
                        .await?;
                    ensure!(
                        answer["blockCID"].as_str() == Some(&built.block_cid),
                        "child push returned another block CID"
                    );
                    pushed = Some(
                        answer["status"]
                            .as_str()
                            .context("child push has no status")?
                            .to_owned(),
                    );
                    emit(answer);
                }
                let mut next = step(
                    pushed.as_deref(),
                    None,
                    journal.built.as_ref(),
                    journal.accepted,
                )?;
                if next == Step::Read {
                    let read: ReadCommitment = serde_json::from_value(
                        author
                            .request(reqwest::Method::GET, "/child-blocks/commitment", None)
                            .await?,
                    )?;
                    // Admission acknowledgment is not persistence: after a parent
                    // restart/eviction, retry the SAME prepared transaction if its
                    // nonce remains unused and no pending spend blocks discovery.
                    if journal.accepted
                        && journal
                            .built
                            .as_ref()
                            .is_some_and(|built| built.parent_state == read.parent_state)
                        && let Some(prepared) = &journal.prepared
                        && parent
                            .nonce(signer.address())
                            .await
                            .is_ok_and(|nonce| nonce == prepared.body.nonce)
                    {
                        journal.accepted = false;
                        files::write(&journal_path, &serde_json::to_vec(&journal)?, false)?;
                    }
                    next = step(
                        pushed.as_deref(),
                        Some(&read),
                        journal.built.as_ref(),
                        journal.accepted,
                    )?;
                }
                if next == Step::Done {
                    journal.done = true;
                    files::write(&journal_path, &serde_json::to_vec(&journal)?, false)?;
                    return Ok::<_, anyhow::Error>(true);
                }
                if next == Step::Build {
                    let built: Built = serde_json::from_value(
                        author
                            .request(
                                reqwest::Method::POST,
                                "/child-blocks/build",
                                Some(json!({"chainPath":path,"recipient":signer.address()})),
                            )
                            .await?,
                    )?;
                    ensure!(
                        standard_cid(&built.block_cid) && standard_cid(&built.parent_state),
                        "invalid child build identifiers"
                    );
                    ensure!(
                        built.submit["chainPath"] == json!(path),
                        "child build path mismatch"
                    );
                    journal.built = Some(built);
                    journal.accepted = false;
                    journal.prepared = None;
                    files::write(&journal_path, &serde_json::to_vec(&journal)?, false)?;
                    next = Step::Commit;
                }
                if next == Step::Commit {
                    if journal.prepared.is_none() {
                        let built = journal.built.as_ref().context("missing built candidate")?;
                        let nonce = parent.nonce(signer.address()).await?;
                        let fee = match options.fee {
                            Some(fee) => fee,
                            None => parent.fee().await?,
                        };
                        let body = transaction::compose(
                            &signer,
                            ActionPlan::CommitChild {
                                directory: directory.clone(),
                                block: built.block_cid.clone(),
                                parent_state: built.parent_state.clone(),
                                previous: built.previous.clone(),
                            },
                            &path[..path.len() - 1].join("/"),
                            nonce,
                            fee,
                        )?;
                        let (signatures, _) = transaction::signed(&signer, &body)?;
                        journal.prepared = Some(Prepared { body, signatures });
                        files::write(&journal_path, &serde_json::to_vec(&journal)?, false)?;
                    }
                    let prepared = journal.prepared.as_ref().unwrap();
                    // Retry precisely the saved signed content, never another nonce.
                    let submitted = HttpTransactionSubmitter::new(
                        &parent.transaction_endpoint()?,
                        Some(parent.options()?),
                    )?
                    .submit_signed(&prepared.signatures, &prepared.body, &cancel)
                    .await?;
                    journal.accepted = true;
                    files::write(&journal_path, &serde_json::to_vec(&journal)?, false)?;
                    emit(
                        json!({"event":"childCommitSubmitted","transactionCID":submitted.transaction_cid,"nonce":prepared.body.nonce.to_string()}),
                    );
                }
                Ok(false)
            };
            let result = tokio::select! {biased;_=cancel.cancelled()=>{emit(json!({"event":"childCommitCancelled","journalRetained":true}));return Ok(())},result=iteration=>result};
            match result {
                Ok(true) => return Ok(()),
                Ok(false) => {}
                Err(error) => emit(json!({"event":"childCommitRetry","error":error.to_string()})),
            }
            tokio::select! {biased;_=cancel.cancelled()=>return Ok(()),_=tokio::time::sleep(Duration::from_millis(options.poll_ms))=>{}}
        }
    };
    let result = match options.timeout_seconds {
        Some(seconds) => tokio::time::timeout(Duration::from_secs(seconds), operation)
            .await
            .unwrap_or_else(|_| {
                Err(anyhow::anyhow!(
                    "child commit timed out; journal retained for resume"
                ))
            }),
        None => operation.await,
    };
    signals.abort();
    result
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn commit_steps_preserve_carried_candidates() {
        let built = Built {
            block_cid: "block".into(),
            parent_state: "state".into(),
            previous: None,
            submit: json!({}),
        };
        let read = ReadCommitment {
            parent_state: "state".into(),
            value: None,
        };
        assert_eq!(
            step(Some("tree"), None, Some(&built), false).unwrap(),
            Step::Done
        );
        for status in ["pending", "awaiting"] {
            assert_eq!(
                step(Some(status), None, Some(&built), false).unwrap(),
                Step::Wait
            );
        }
        assert_eq!(
            step(Some("dropped"), None, Some(&built), false).unwrap(),
            Step::Read
        );
        assert_eq!(
            step(None, Some(&read), Some(&built), false).unwrap(),
            Step::Commit
        );
        assert_eq!(
            step(None, Some(&read), Some(&built), true).unwrap(),
            Step::Wait
        );
        let advanced = ReadCommitment {
            parent_state: "next".into(),
            value: Some("block@state".into()),
        };
        assert_eq!(
            step(Some("dropped"), Some(&advanced), Some(&built), false).unwrap(),
            Step::Wait
        );
        assert_eq!(step(None, Some(&read), None, false).unwrap(), Step::Build);
        assert!(step(Some("invented"), None, None, false).is_err());
    }
    #[tokio::test]
    async fn invalid_create_preserves_existing_topology_and_files() {
        let temp = tempfile::tempdir().unwrap();
        crate::lifecycle::init(temp.path(), 4001, 8080, true).unwrap();
        let original = std::fs::read(temp.path().join("lattice.json")).unwrap();
        let create = Create {
            path: "Nexus/Alpha".into(),
            spec: None,
            policy_modules: vec![],
            block_time: 1000,
            reward: 10,
            premine: 1,
            premine_to: None,
            genesis_transactions: None,
        };
        assert!(super::create(temp.path(), create).await.is_err());
        assert_eq!(
            std::fs::read(temp.path().join("lattice.json")).unwrap(),
            original
        );
        assert!(!temp.path().join("specs").exists());
    }
}
