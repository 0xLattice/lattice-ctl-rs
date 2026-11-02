//! Local operator lifecycle. PID records include kernel process birth identity.
mod config;
mod files;
mod process;
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
async fn stop(record: &Record) -> Result<bool> {
    if !process::signal(&record.process, libc::SIGTERM)? {
        return Ok(false);
    }
    let deadline = Instant::now() + Duration::from_secs(30);
    while process::matches(&record.process)? {
        if Instant::now() >= deadline {
            process::signal(&record.process, libc::SIGKILL)?;
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let deadline = Instant::now() + Duration::from_secs(5);
    while process::matches(&record.process)? {
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
    let stopped = stop(&record).await?;
    files::remove_if_unchanged(&layout.pid(), &bytes)?;
    Ok(json!({"stopped":stopped}))
}
pub async fn up(root: &Path, node_binary: &Path, listen_host: IpAddr) -> Result<Value> {
    let layout = Layout::open(root)?;
    let _lock = layout.lock()?;
    for directory in ["log", "chains/Nexus", "specs"] {
        layout.ensure(directory)?;
    }
    let public_key = files::identity(&layout, false)?;
    let topology = Topology::load(&layout.root)?;
    let (config, config_fingerprint) = config::build(&layout, &topology, listen_host)?;
    // Public identity binds key changes without persisting private material.
    let fingerprint = lattice_sdk::primitives::crypto::sha256(&serde_json::to_string(
        &json!({"configuration":config_fingerprint,"publicKey":public_key}),
    )?);
    let binary = node_binary
        .canonicalize()
        .context("locating lattice-node binary")?;
    if layout.pid().exists() {
        let bytes = files::read(&layout.pid(), 1024 * 1024)?;
        let record: Record = serde_json::from_slice(&bytes)?;
        if process::matches(&record.process)? {
            if record.fingerprint == fingerprint && Path::new(&record.process.executable) == binary
            {
                return Ok(json!({"running":true,"started":false,"pid":record.process.pid}));
            }
            stop(&record).await?;
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
    let mut child = starting.0.take().unwrap();
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(json!({"running":true,"started":true,"pid":pid}))
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
