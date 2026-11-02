//! Local operator lifecycle. PID records include kernel process birth identity.
pub(crate) mod config;
pub(crate) mod files;
pub(crate) mod process;
use crate::topology::{ChainPeers, Topology};
use anyhow::{Context, Result, ensure};
use files::Layout;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::os::unix::{fs::OpenOptionsExt, process::CommandExt};
use std::{
    net::IpAddr,
    path::Path,
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};
#[derive(Deserialize, Serialize)]
struct Record {
    #[serde(default)]
    public_key: Option<String>,
    process: process::ProcessIdentity,
    config: Value,
    fingerprint: String,
}
struct Starting(Option<Child>);
impl Drop for Starting {
    fn drop(&mut self) {
        if let Some(child) = self.0.as_mut() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}
pub fn init(root: &Path, listen: u16, rpc: u16, no_default_peers: bool) -> Result<Value> {
    let topology = Topology {
        listen,
        rpc,
        peers: no_default_peers.then(|| ChainPeers::Nexus(vec![])),
        ..Default::default()
    };
    topology.validate()?;
    ensure!(!root.is_symlink(), "host root cannot be a symlink");
    std::fs::create_dir_all(root)?;
    let layout = Layout::open(root)?;
    let _lock = layout.lock()?;
    ensure!(
        !layout.root.join("lattice.json").exists(),
        "topology already exists"
    );
    // Install exclusions before any secret is generated. Existing exclusions are
    // preserved only when they already match the required managed-directory guard.
    for directory in ["identity", "chains", "run", "log"] {
        let path = layout.ensure(directory)?.join(".gitignore");
        let guard = b"*\n!.gitignore\n";
        if path.exists() {
            ensure!(
                files::read(&path, 4096)? == guard,
                "managed directory has incompatible .gitignore"
            );
        } else {
            files::write(&path, guard, true)?;
        }
    }
    let public_key = files::identity(&layout, true)?;
    let genesis = serde_json::to_vec(&crate::bootstrap::volumes()?)?;
    let path = layout.root.join("genesis.json");
    if path.exists() {
        ensure!(
            files::read(&path, 16 * 1024 * 1024)? == genesis,
            "existing genesis differs from canonical bootstrap"
        );
    } else {
        files::write(&path, &genesis, true)?;
    }
    files::write(
        &layout.root.join("lattice.json"),
        &serde_json::to_vec_pretty(&topology)?,
        true,
    )?;
    Ok(json!({"initialized":true,"publicKey":public_key}))
}
pub fn identity(root: &Path) -> Result<Value> {
    let layout = Layout::open(root)?;
    let _lock = layout.lock()?;
    Ok(json!({"publicKey":files::identity(&layout, false)?}))
}
pub(crate) async fn stop_process(identity: &process::ProcessIdentity) -> Result<bool> {
    if !process::signal(identity, libc::SIGTERM)? {
        return Ok(false);
    }
    let deadline = Instant::now() + Duration::from_secs(30);
    while process::matches(identity)? {
        if Instant::now() >= deadline {
            process::signal(identity, libc::SIGKILL)?;
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let deadline = Instant::now() + Duration::from_secs(5);
    while process::matches(identity)? {
        ensure!(Instant::now() < deadline, "process did not stop");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    Ok(true)
}
pub async fn down(root: &Path) -> Result<Value> {
    let layout = Layout::open(root)?;
    let _lock = layout.lock()?;
    if !layout.pid().exists() {
        return Ok(json!({"stopped":false}));
    }
    let bytes = files::read(&layout.pid(), 1024 * 1024)?;
    let record: Record = serde_json::from_slice(&bytes)?;
    let stopped = stop_process(&record.process).await?;
    files::remove_if_unchanged(&layout.pid(), &bytes)?;
    Ok(json!({"stopped":stopped}))
}
pub async fn up(root: &Path, node_binary: &Path, listen_host: IpAddr) -> Result<Value> {
    Ok(up_owned(root, node_binary, listen_host).await?.0)
}
async fn up_owned(
    root: &Path,
    node_binary: &Path,
    listen_host: IpAddr,
) -> Result<(Value, process::ProcessIdentity)> {
    let layout = Layout::open(root)?;
    let _lock = layout.lock()?;
    for directory in ["log", "chains/Nexus", "specs"] {
        layout.ensure(directory)?;
    }
    let public_key = files::identity(&layout, false)?;
    let topology = files::desired_topology(&layout)?;
    let (config, config_fingerprint) = config::build(&layout, &topology, listen_host)?;
    // Public identity binds key changes without persisting private material.
    let fingerprint = lattice_sdk::primitives::crypto::sha256(&serde_json::to_string(
        &json!({"configuration":config_fingerprint,"publicKey":public_key}),
    )?);
    let binary = node_binary
        .canonicalize()
        .context("locating lattice-node binary")?;
    ensure!(
        binary.is_file(),
        "lattice-node binary must be a regular file"
    );
    use std::os::unix::ffi::OsStrExt;
    let executable = std::ffi::CString::new(binary.as_os_str().as_bytes())?;
    ensure!(
        unsafe { libc::access(executable.as_ptr(), libc::X_OK) } == 0,
        "lattice-node binary is not executable"
    );
    if layout.pid().exists() {
        let bytes = files::read(&layout.pid(), 1024 * 1024)?;
        let record: Record = serde_json::from_slice(&bytes)?;
        if process::matches(&record.process)? {
            if record.fingerprint == fingerprint
                && Path::new(&record.process.executable) == binary
                && record.public_key.as_ref() == Some(&public_key)
            {
                files::finish_topology(&layout, &topology)?;
                return Ok((
                    json!({"running":true,"started":false,"pid":record.process.pid}),
                    record.process,
                ));
            }
            stop_process(&record.process).await?;
        }
        files::remove_if_unchanged(&layout.pid(), &bytes)?;
    }
    files::write(
        &layout.config(),
        &serde_json::to_vec_pretty(&config)?,
        false,
    )?;
    let log = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(layout.log())?;
    let offset = log.metadata()?.len();
    let child = Command::new(&binary)
        .arg("--config")
        .arg(layout.config())
        .stdin(Stdio::null())
        .stdout(log.try_clone()?)
        .stderr(log)
        .process_group(0)
        .spawn()?;
    let mut starting = Starting(Some(child));
    let pid = starting.0.as_ref().unwrap().id() as i32;
    let identity = process::inspect(pid)?.context("node exited during startup")?;
    ensure!(
        Path::new(&identity.executable) == binary,
        "spawned executable identity differs"
    );
    let record = Record {
        public_key: Some(public_key),
        process: identity,
        config,
        fingerprint,
    };
    let bytes = serde_json::to_vec(&record)?;
    files::write(&layout.pid(), &bytes, true)?;
    let readiness = async {
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            ensure!(
                starting.0.as_mut().unwrap().try_wait()?.is_none(),
                "node exited during startup; inspect host log"
            );
            ensure!(
                Instant::now() < deadline,
                "node startup timed out; inspect host log"
            );
            // Only newly appended output can acknowledge this spawn.
            use std::io::{Read, Seek};
            let mut file = std::fs::File::open(layout.log())?;
            file.seek(std::io::SeekFrom::Start(offset))?;
            let mut output = String::new();
            file.take(4 * 1024 * 1024).read_to_string(&mut output)?;
            if output
                .lines()
                .filter_map(|line| serde_json::from_str::<Value>(line).ok())
                .any(|v| v["event"] == "ready")
            {
                ensure!(
                    process::matches(&record.process)?,
                    "node exited during startup"
                );
                return Ok::<_, anyhow::Error>(());
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }
    .await;
    if let Err(error) = readiness {
        files::remove_if_unchanged(&layout.pid(), &bytes)?;
        return Err(error);
    }
    files::finish_topology(&layout, &topology)?;
    let mut child = starting.0.take().unwrap();
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok((
        json!({"running":true,"started":true,"pid":pid}),
        record.process,
    ))
}
/// Foreground owner: keep one node alive and stop it on SIGINT/SIGTERM.
pub async fn supervise(root: &Path, node_binary: &Path, listen_host: IpAddr) -> Result<()> {
    let layout = Layout::open(root)?;
    let _lease = files::supervisor_lease(&layout)?;
    let mut terminate = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    let mut interrupt = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt())?;
    let cancel = tokio_util::sync::CancellationToken::new();
    let signal = cancel.clone();
    let signals = tokio::spawn(async move {
        tokio::select! {_=terminate.recv()=>{},_=interrupt.recv()=>{}};
        signal.cancel();
    });
    let mut owned: Option<process::ProcessIdentity> = None;
    let operation=async {
        loop {
            // Dropping an in-progress up kills/reaps its guarded startup child.
            let (result,identity)=tokio::select!{biased;_=cancel.cancelled()=>return Ok::<(),anyhow::Error>(()),result=up_owned(root,node_binary,listen_host)=>result?};
            owned=Some(identity);
            println!("{}",json!({"event":"supervisorReady","node":result}));
            loop {
                tokio::select!{biased;_=cancel.cancelled()=>return Ok(()),_=tokio::time::sleep(Duration::from_secs(1))=>{}}
                let alive={
                    let _lock=layout.lock()?;
                    if layout.pid().exists(){
                        let record:Record=serde_json::from_slice(&files::read(&layout.pid(),1024*1024)?)?;
                        if process::matches(&record.process)? {owned=Some(record.process);true}
                        else {owned.as_ref().map(process::matches).transpose()?.unwrap_or(false)}
                    }else{owned.as_ref().map(process::matches).transpose()?.unwrap_or(false)}
                };
                if !alive {println!("{}",json!({"event":"nodeExited","action":"restart"}));break;}
            }
        }
    }.await;
    // Memory retains the actual process birth identity even if the PID file
    // disappears or becomes unreadable. The lease outlives final cleanup.
    let stopped = async {
        // A failed initial up has not adopted any daemon. Never stop a
        // pre-existing node just because validation or binary lookup failed.
        let Some(owned) = &owned else {
            return Ok::<(), anyhow::Error>(());
        };
        let _lock = layout.lock()?;
        stop_process(owned).await?;
        if layout.pid().exists() {
            let bytes = files::read(&layout.pid(), 1024 * 1024)?;
            let record: Record = serde_json::from_slice(&bytes)?;
            // A replacement not observed/adopted by this supervisor is not ours.
            if record.process == *owned {
                files::remove_if_unchanged(&layout.pid(), &bytes)?;
            }
        }
        Ok::<(), anyhow::Error>(())
    }
    .await;
    signals.abort();
    match (operation, stopped) {
        (Err(error), Err(stop)) => {
            return Err(error.context(format!("shutdown also failed: {stop}")));
        }
        (Err(error), _) | (_, Err(error)) => return Err(error),
        _ => {}
    }
    println!("{}", json!({"event":"supervisorStopped"}));
    Ok(())
}
/// Remove only the stopped tree's chain storage, preserving identity/configuration.
pub fn wipe(root: &Path) -> Result<Value> {
    use fs2::FileExt;
    let layout = Layout::open(root)?;
    let _spawn = layout.lock()?;
    let _supervisor = files::supervisor_lease(&layout)?;
    files::desired_topology(&layout)?.validate()?;
    if layout.pid().exists() {
        let record: Record = serde_json::from_slice(&files::read(&layout.pid(), 1024 * 1024)?)?;
        ensure!(
            !process::matches(&record.process)?,
            "node is running; stop it before wipe"
        );
    }
    let container = layout.root.join("chains");
    let directory = container.join("Nexus");
    ensure!(
        !container.is_symlink() && !directory.is_symlink(),
        "refusing symlinked chain storage"
    );
    if !directory.exists() {
        return Ok(json!({"wiped":false}));
    }
    ensure!(
        directory.is_dir() && container.canonicalize()? == layout.root.join("chains"),
        "unexpected chain storage location"
    );
    // This is the same lock file/inode used by NodeStore, even if PID records
    // were removed or corrupted. O_NOFOLLOW refuses substituted lock symlinks.
    let writer = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(directory.join(".lattice-node.lock"))?;
    writer
        .try_lock_exclusive()
        .context("chain storage is in use; stop its writer before wipe")?;
    // Detach the old directory atomically; a later external node opening the
    // original path cannot have its newly created storage swept by this removal.
    let detached = tempfile::Builder::new()
        .prefix(".wipe-")
        .tempdir_in(&container)?;
    std::fs::rename(&directory, detached.path())?;
    std::fs::File::open(&container)?.sync_all()?;
    detached.close().context(
        "detached storage could not be fully removed; inspect chains/.wipe-* before retry",
    )?;
    std::fs::File::open(&container)?.sync_all()?;
    drop(writer);
    Ok(json!({"wiped":true,"preserved":["identity","topology","specs","journals"]}))
}

/// Apply only hosted-chain changes to a live owned node while caller holds lock.
pub(crate) async fn reconcile_hosted(
    layout: &files::Layout,
    topology: &crate::topology::Topology,
) -> Result<bool> {
    if !layout.pid().exists() {
        return Ok(false);
    }
    let bytes = files::read(&layout.pid(), 1024 * 1024)?;
    let mut record: Record = serde_json::from_slice(&bytes)?;
    if !process::matches(&record.process)? {
        return Ok(false);
    }
    let listen: std::net::SocketAddr = record.config["listen"]
        .as_str()
        .context("recorded listen address missing")?
        .parse()?;
    let (desired, config_fingerprint) = config::build(layout, topology, listen.ip())?;
    let fields = [
        "hostChain",
        "premineTo",
        "genesisTransactions",
        "chainPeer",
        "policyModule",
    ];
    let static_config = |mut value: Value| {
        for field in fields {
            value.as_object_mut().unwrap().remove(field);
        }
        value
    };
    ensure!(
        static_config(record.config.clone()) == static_config(desired.clone()),
        "non-hosted configuration changed; run up to restart before applying children"
    );
    let public_key = files::identity(layout, false)?;
    ensure!(
        record.public_key.as_ref() == Some(&public_key),
        "overlay identity changed or legacy process record; run up before child apply"
    );
    let fingerprint = lattice_sdk::primitives::crypto::sha256(&serde_json::to_string(
        &json!({"configuration":config_fingerprint,"publicKey":public_key}),
    )?);
    let (body, _) = config::hosted(layout, topology)?;
    let rpc = crate::rpc::LocalRpc::new(
        &format!("http://127.0.0.1:{}", topology.rpc),
        layout.cookie(),
        "Nexus",
    )?;
    rpc.request(reqwest::Method::POST, "/hosted-chains", Some(body))
        .await?;
    ensure!(
        process::matches(&record.process)?,
        "node changed during hosted configuration reconciliation"
    );
    record.config = desired;
    record.fingerprint = fingerprint;
    files::write(
        &layout.config(),
        &serde_json::to_vec_pretty(&record.config)?,
        false,
    )?;
    files::write(&layout.pid(), &serde_json::to_vec(&record)?, false)?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn initialization_is_private_and_never_overwrites() {
        use std::os::unix::fs::PermissionsExt;
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("host");
        let first = init(&root, 4001, 8080, true).unwrap();
        assert_eq!(first["publicKey"], identity(&root).unwrap()["publicKey"]);
        assert_eq!(
            std::fs::metadata(root.join("identity/Nexus.key"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
        assert!(init(&root, 4002, 8081, true).is_err());
        assert_eq!(Topology::load(&root).unwrap().listen, 4001);
    }
    #[tokio::test]
    async fn stale_birth_never_signals_live_process() {
        let temp = tempfile::tempdir().unwrap();
        let layout = Layout::open(temp.path()).unwrap();
        layout.ensure("run").unwrap();
        let mut identity = process::inspect(std::process::id() as i32)
            .unwrap()
            .unwrap();
        identity.birth.push_str("-stale");
        let record = Record {
            public_key: None,
            process: identity,
            config: json!({}),
            fingerprint: String::new(),
        };
        files::write(&layout.pid(), &serde_json::to_vec(&record).unwrap(), true).unwrap();
        assert_eq!(down(temp.path()).await.unwrap()["stopped"], false);
        assert!(!layout.pid().exists());
    }
    #[test]
    fn preserves_swift_url_field_spelling() {
        let topology: Topology = serde_json::from_value(
            json!({"listen":4001,"rpc":8080,"publicReadURL":"https://example.invalid"}),
        )
        .unwrap();
        topology.validate().unwrap();
        assert_eq!(
            serde_json::to_value(topology).unwrap()["publicReadURL"],
            "https://example.invalid"
        );
    }
    #[test]
    fn rejects_managed_symlink() {
        let temp = tempfile::tempdir().unwrap();
        std::os::unix::fs::symlink(temp.path(), temp.path().join("run")).unwrap();
        assert!(Layout::open(temp.path()).unwrap().lock().is_err());
    }
    #[test]
    fn topology_rejects_unhosted_peer_and_unsupported_transport() {
        let mut topology = Topology {
            peers: Some(ChainPeers::ByChain(std::collections::BTreeMap::from([(
                "Nexus/Missing".into(),
                vec![],
            )]))),
            ..Default::default()
        };
        assert!(topology.validate().is_err());
        topology.peers = None;
        topology.web_socket = Some(9000);
        assert!(topology.validate().is_ok());
        assert!(topology.ensure_daemon_supported().is_err());
    }
}
