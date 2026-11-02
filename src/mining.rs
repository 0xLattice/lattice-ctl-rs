//! Operator mining loop over the shared production coordinator and workers.
use crate::{
    lifecycle::{
        files::{self, Layout},
        process, stop_process,
    },
    topology::{Topology, TopologyMine},
};
use anyhow::{Context, Result, ensure};
use fs2::FileExt;
use lattice_mining::{
    Coordinator, CoordinatorOptions, CpuWorker, Cycle, HttpNodeClient, NodeClient, ProcessWorker,
    Worker, template_request,
};
use serde_json::{Value, json};
use std::{
    fs::{File, OpenOptions},
    io::{Read, Seek, Write},
    os::unix::{
        fs::{OpenOptionsExt, PermissionsExt},
        process::CommandExt,
    },
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::Arc,
    time::{Duration, Instant},
};
use tokio_util::sync::CancellationToken;

struct Settings {
    mine: TopologyMine,
    request: Value,
    worker: Option<PathBuf>,
    rpc: u16,
    workers: usize,
}
fn settings(layout: &Layout) -> Result<Settings> {
    let topology = Topology::load(&layout.root)?;
    let mine = topology.mine.context("no mine section in lattice.json")?;
    let workers = usize::try_from(mine.workers.unwrap_or(1))?;
    ensure!(
        (1..=1024).contains(&workers),
        "mine.workers must be between 1 and 1024"
    );
    let entries = |map: &Option<std::collections::BTreeMap<String, String>>| {
        map.iter()
            .flat_map(|m| m.iter())
            .map(|(path, value)| format!("{path}={value}"))
            .collect::<Vec<_>>()
    };
    let request = template_request(&entries(&mine.recipients), &entries(&mine.min_work))?;
    for value in mine.recipients.iter().flat_map(|m| m.values()) {
        ensure!(
            lattice_sdk::primitives::standard_cid(value),
            "mining recipient must be a standard Lattice address"
        );
    }
    let worker = match mine.worker.as_deref() {
        None | Some("cpu") => None,
        Some(path) => {
            let path = Path::new(path);
            let path = if path.is_absolute() {
                path.to_owned()
            } else {
                layout.root.join(path)
            }
            .canonicalize()
            .context("locating mining worker")?;
            ensure!(
                path.is_file() && path.metadata()?.permissions().mode() & 0o111 != 0,
                "mining worker must be executable"
            );
            Some(path)
        }
    };
    Ok(Settings {
        mine,
        request,
        worker,
        rpc: topology.rpc,
        workers,
    })
}
fn pid(layout: &Layout) -> PathBuf {
    layout.root.join("run/mine.pid")
}
fn log(layout: &Layout) -> PathBuf {
    layout.root.join("log/mine.log")
}
fn running(layout: &Layout) -> Result<Option<process::ProcessIdentity>> {
    let path = pid(layout);
    if !path.exists() {
        return Ok(None);
    }
    let identity = serde_json::from_slice(&files::read(&path, 4096)?)?;
    Ok(process::matches(&identity)?.then_some(identity))
}
fn owner(layout: &Layout) -> Result<File> {
    layout.ensure("run")?;
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(layout.root.join("run/mine-owner.lock"))?;
    file.try_lock_exclusive()
        .context("another mining loop owns this host")?;
    Ok(file)
}
fn event(value: Value) -> Result<()> {
    println!("{value}");
    std::io::stdout().flush()?;
    Ok(())
}
fn deadline(expiry: Duration, longest: Duration, multiplier: i64) -> Duration {
    expiry
        .saturating_add(longest.min(expiry))
        .checked_mul(u32::try_from(multiplier).unwrap_or(u32::MAX))
        .unwrap_or(Duration::MAX)
}
async fn hold(duration: Duration, cancel: &CancellationToken) {
    tokio::select! {biased;_=cancel.cancelled()=>{},_=tokio::time::sleep(duration)=>{}}
}

/// Run on the caller's cancellation token; no wallet key or signing is needed.
async fn mining_loop(
    settings: Settings,
    layout: &Layout,
    once: bool,
    cancel: &CancellationToken,
) -> Result<()> {
    let node = Arc::new(HttpNodeClient::new(
        &format!("http://127.0.0.1:{}", settings.rpc),
        settings.request,
        Some(layout.cookie()),
        Duration::from_secs(settings.mine.template_timeout_seconds.unwrap_or(15)),
        16 * 1024 * 1024,
    )?);
    let workers: Vec<Arc<dyn Worker>> = (0..settings.workers)
        .map(|i| match &settings.worker {
            Some(path) => {
                Arc::new(ProcessWorker::new(format!("worker-{i}"), path.clone())) as Arc<dyn Worker>
            }
            None => Arc::new(CpuWorker::new(format!("cpu-{i}"))) as Arc<dyn Worker>,
        })
        .collect();
    let mut coordinator = Coordinator::new(
        node.clone(),
        workers,
        CoordinatorOptions {
            total_batch_size: settings.mine.batch_size.unwrap_or(2_000_000_000),
            ..Default::default()
        },
    );
    let mut longest = Duration::ZERO;
    event(json!({"event":"mineReady"}))?;
    loop {
        // Probe with the same recipient/minimum-work plan as the coordinator.
        let observed =
            tokio::select! {biased;_=cancel.cancelled()=>break,result=node.fetch_work()=>result};
        let expiry = observed
            .ok()
            .flatten()
            .and_then(|work| work.lifetime)
            .filter(|expiry| !expiry.is_zero());
        let Some(expiry) = expiry else {
            event(json!({"event":"notMining","reason":"no usable template expiry"}))?;
            if once {
                anyhow::bail!("node provided no usable template expiry")
            }
            hold(Duration::from_secs(5), cancel).await;
            continue;
        };
        let bound = deadline(
            expiry,
            longest,
            settings.mine.round_deadline_multiplier.unwrap_or(10),
        );
        let started = Instant::now();
        let round = cancel.child_token();
        let result = tokio::time::timeout(bound, coordinator.run_batch(&round)).await;
        round.cancel();
        let cycle = match result {
            Ok(cycle) => cycle,
            Err(_) => {
                event(
                    json!({"event":"roundDeadlineExceeded","milliseconds":bound.as_millis().min(u64::MAX as u128) as u64}),
                )?;
                if once {
                    anyhow::bail!("mining round deadline exceeded")
                }
                hold(Duration::from_secs(5), cancel).await;
                continue;
            }
        };
        let elapsed = started.elapsed();
        if matches!(
            cycle,
            Cycle::NoSolution { .. } | Cycle::Stale { .. } | Cycle::Submitted { .. }
        ) {
            longest = longest.max(elapsed);
        }
        event(serde_json::to_value(&cycle)?)?;
        if matches!(cycle, Cycle::Cancelled) || cancel.is_cancelled() {
            break;
        }
        let trouble = matches!(cycle, Cycle::NodeFailed { .. } | Cycle::WorkerFailed { .. });
        if once {
            ensure!(!trouble, "mining round failed");
            break;
        }
        let accepted = matches!(
            cycle,
            Cycle::Submitted {
                submission: lattice_mining::Submission { accepted: true, .. },
                ..
            }
        );
        if accepted {
            hold(
                Duration::from_secs(settings.mine.min_block_interval_seconds.unwrap_or(0))
                    .saturating_sub(elapsed),
                cancel,
            )
            .await;
        } else if trouble || matches!(cycle, Cycle::Backoff | Cycle::Submitted { .. }) {
            hold(Duration::from_secs(5), cancel).await;
        }
    }
    event(json!({"event":"mineStopped","metrics":coordinator.metrics()}))?;
    Ok(())
}

pub async fn run(root: &Path, once: bool, managed: bool) -> Result<()> {
    let layout = Layout::open(root)?;
    let settings = settings(&layout)?;
    // Register handlers before acknowledging readiness to the spawning CLI.
    let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    let mut interrupt = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt())?;
    let cancel = CancellationToken::new();
    let signal = cancel.clone();
    let signals = tokio::spawn(async move {
        tokio::select! {_=term.recv()=>{},_=interrupt.recv()=>{}};
        signal.cancel();
    });
    let _owner = owner(&layout)?;
    let identity =
        process::inspect(std::process::id() as i32)?.context("cannot identify mining process")?;
    let bytes = serde_json::to_vec(&identity)?;
    if !managed {
        let _lock = layout.lock()?;
        ensure!(running(&layout)?.is_none(), "mining already running");
        files::write(&pid(&layout), &bytes, false)?;
    }
    let result = mining_loop(settings, &layout, once, &cancel).await;
    signals.abort();
    // No spawn lock is required: compare-before-remove only targets this owner,
    // while the exclusive lease still excludes another loop.
    files::remove_if_unchanged(&pid(&layout), &bytes)?;
    result
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
pub async fn start(root: &Path) -> Result<Value> {
    let layout = Layout::open(root)?;
    let _lock = layout.lock()?;
    settings(&layout)?;
    ensure!(running(&layout)?.is_none(), "mining already running");
    // Refuse a foreground miner even before its PID registration is visible.
    {
        let _lease = owner(&layout)?;
    }
    layout.ensure("log")?;
    let path = pid(&layout);
    if path.exists() {
        std::fs::remove_file(&path)?;
    }
    let output = OpenOptions::new()
        .create(true)
        .append(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(log(&layout))?;
    let offset = output.metadata()?.len();
    let binary = std::env::current_exe()?.canonicalize()?;
    let child = Command::new(&binary)
        .arg("--root")
        .arg(&layout.root)
        .args(["mine", "run", "--managed"])
        .stdin(Stdio::null())
        .stdout(output.try_clone()?)
        .stderr(output)
        .process_group(0)
        .spawn()?;
    let mut starting = Starting(Some(child));
    let identity = process::inspect(starting.0.as_ref().unwrap().id() as i32)?
        .context("mining loop exited during startup")?;
    ensure!(
        Path::new(&identity.executable) == binary,
        "mining executable identity differs"
    );
    let bytes = serde_json::to_vec(&identity)?;
    files::write(&path, &bytes, true)?;
    let ready = async {
        let until = Instant::now() + Duration::from_secs(10);
        loop {
            ensure!(
                starting.0.as_mut().unwrap().try_wait()?.is_none(),
                "mining loop exited; inspect mine.log"
            );
            ensure!(Instant::now() < until, "mining startup timed out");
            let mut file = File::open(log(&layout))?;
            file.seek(std::io::SeekFrom::Start(offset))?;
            let mut text = String::new();
            file.take(1024 * 1024).read_to_string(&mut text)?;
            if text
                .lines()
                .filter_map(|l| serde_json::from_str::<Value>(l).ok())
                .any(|v| v["event"] == "mineReady")
            {
                return Ok::<_, anyhow::Error>(());
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    }
    .await;
    if let Err(error) = ready {
        files::remove_if_unchanged(&path, &bytes)?;
        return Err(error);
    }
    let mut child = starting.0.take().unwrap();
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(json!({"running":true,"pid":identity.pid}))
}
pub async fn stop(root: &Path) -> Result<Value> {
    let layout = Layout::open(root)?;
    let _lock = layout.lock()?;
    let path = pid(&layout);
    if !path.exists() {
        return Ok(json!({"stopped":false}));
    }
    let bytes = files::read(&path, 4096)?;
    let identity = serde_json::from_slice(&bytes)?;
    let stopped = stop_process(&identity).await?;
    files::remove_if_unchanged(&path, &bytes)?;
    Ok(json!({"stopped":stopped}))
}
pub fn status(root: &Path) -> Result<Value> {
    let layout = Layout::open(root)?;
    let _lock = layout.lock()?;
    let settings = settings(&layout)?;
    Ok(
        json!({"running":running(&layout)?.map(|p|p.pid),"plan":settings.request,"workers":settings.workers,"worker":if settings.worker.is_some(){"process"}else{"cpu"},"rewardsWithoutRecipient":"burn","planAppliesOn":"next run/start; stop and start to reload"}),
    )
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn round_bound_clamps_completed_time() {
        assert_eq!(
            deadline(Duration::from_secs(30), Duration::from_secs(3000), 10),
            Duration::from_secs(600)
        );
    }
    #[test]
    fn invalid_plan_fails_before_worker_spawn() {
        let temp = tempfile::tempdir().unwrap();
        let mut topology = serde_json::to_value(Topology::default()).unwrap();
        topology["mine"] = json!({"recipients":{"Nexus":"invalid"},"minWork":{"Nexus":"2^8"}});
        std::fs::write(
            temp.path().join("lattice.json"),
            serde_json::to_vec(&topology).unwrap(),
        )
        .unwrap();
        assert!(settings(&Layout::open(temp.path()).unwrap()).is_err());
        topology["mine"] = json!({"minWork":{"Nexus":"0"}});
        std::fs::write(
            temp.path().join("lattice.json"),
            serde_json::to_vec(&topology).unwrap(),
        )
        .unwrap();
        assert!(settings(&Layout::open(temp.path()).unwrap()).is_err());
    }
}
