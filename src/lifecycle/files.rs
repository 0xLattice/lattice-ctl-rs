use anyhow::{Result, ensure};
use fs2::FileExt;
use std::{
    fs::{File, OpenOptions},
    io::{Read, Write},
    os::unix::fs::{OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
};
#[derive(Clone)]
pub struct Layout {
    pub root: PathBuf,
}
impl Layout {
    pub fn open(root: &Path) -> Result<Self> {
        ensure!(!root.is_symlink(), "host root cannot be a symlink");
        Ok(Self {
            root: root.canonicalize()?,
        })
    }
    pub fn ensure(&self, relative: &str) -> Result<PathBuf> {
        let mut at = self.root.clone();
        for part in relative.split('/') {
            ensure!(
                !part.is_empty() && part != "." && part != "..",
                "invalid managed directory"
            );
            at.push(part);
            ensure!(!at.is_symlink(), "managed directory cannot be a symlink");
            std::fs::create_dir_all(&at)?;
        }
        Ok(at)
    }
    pub fn pid(&self) -> PathBuf {
        self.root.join("run/lattice-node.pid")
    }
    pub fn config(&self) -> PathBuf {
        self.root.join("run/node-config.json")
    }
    pub fn cookie(&self) -> PathBuf {
        self.root.join("chains/Nexus/.cookie")
    }
    pub fn log(&self) -> PathBuf {
        self.root.join("log/lattice-node.log")
    }
    pub fn identity(&self) -> PathBuf {
        self.root.join("identity/Nexus.key")
    }
    pub fn lock(&self) -> Result<SpawnLock> {
        let run = self.ensure("run")?;
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW)
            .open(run.join("spawn-lock.pid"))?;
        file.lock_exclusive()?;
        Ok(SpawnLock(file))
    }
}
pub struct SpawnLock(File);
impl Drop for SpawnLock {
    fn drop(&mut self) {
        let _ = FileExt::unlock(&self.0);
    }
}
pub fn read(path: &Path, limit: u64) -> Result<Vec<u8>> {
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)?;
    ensure!(file.metadata()?.is_file(), "managed file must be regular");
    let mut bytes = vec![];
    file.take(limit + 1).read_to_end(&mut bytes)?;
    ensure!(bytes.len() as u64 <= limit, "managed file exceeds limit");
    Ok(bytes)
}
pub fn write(path: &Path, bytes: &[u8], exclusive: bool) -> Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("managed file has no parent"))?;
    ensure!(!path.is_symlink(), "managed file cannot be a symlink");
    let mut temp = tempfile::NamedTempFile::new_in(parent)?;
    temp.as_file()
        .set_permissions(std::fs::Permissions::from_mode(0o600))?;
    temp.write_all(bytes)?;
    temp.as_file().sync_all()?;
    if exclusive {
        temp.persist_noclobber(path).map_err(|e| e.error)?;
    } else {
        temp.persist(path).map_err(|e| e.error)?;
    }
    File::open(parent)?.sync_all()?;
    Ok(())
}
pub fn remove_if_unchanged(path: &Path, expected: &[u8]) -> Result<()> {
    if read(path, 1024 * 1024).is_ok_and(|bytes| bytes == expected) {
        std::fs::remove_file(path)?;
    }
    Ok(())
}
pub fn identity(layout: &Layout, create: bool) -> Result<String> {
    use rand_core::RngCore;
    layout.ensure("identity")?;
    let path = layout.identity();
    if !path.exists() {
        ensure!(
            create,
            "overlay identity is missing; restore the original key before starting"
        );
        let mut seed = [0; 32];
        rand_core::OsRng.fill_bytes(&mut seed);
        write(&path, format!("{}\n", hex::encode(seed)).as_bytes(), true)?;
    }
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(&path)?;
    ensure!(
        file.metadata()?.permissions().mode() & 0o077 == 0,
        "identity is group/other-readable; chmod 600 it"
    );
    let bytes = read(&path, 1024)?;
    let value = std::str::from_utf8(&bytes)?.trim();
    ensure!(
        value.len() == 64,
        "identity must contain a 32-byte private key"
    );
    let seed: [u8; 32] = hex::decode(value)?
        .try_into()
        .map_err(|_| anyhow::anyhow!("invalid identity length"))?;
    let account = lattice_sdk::transactions::SigningAccount::from_private_key(&seed)?;
    Ok(account
        .public_key()
        .strip_prefix("ed01")
        .ok_or_else(|| anyhow::anyhow!("invalid Ed25519 public key"))?
        .into())
}

#[derive(serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct PendingTopology {
    version: u8,
    operation: String,
    topology: crate::topology::Topology,
}
pub fn pending_path(layout: &Layout) -> PathBuf {
    layout.root.join("run/pending-topology.json")
}
pub fn ensure_no_pending(layout: &Layout) -> Result<()> {
    ensure!(
        !pending_path(layout).exists() && !pending_path(layout).is_symlink(),
        "a child configuration is pending; finish it with child apply or up"
    );
    Ok(())
}
pub fn stage_topology(layout: &Layout, topology: &crate::topology::Topology) -> Result<()> {
    topology.validate()?;
    layout.ensure("run")?;
    write(
        &pending_path(layout),
        &serde_json::to_vec_pretty(&PendingTopology {
            version: 1,
            operation: "apply-hosted-chains".into(),
            topology: topology.clone(),
        })?,
        true,
    )
}
pub fn desired_topology(layout: &Layout) -> Result<crate::topology::Topology> {
    if !pending_path(layout).exists() {
        return crate::topology::Topology::load(&layout.root);
    }
    let pending: PendingTopology =
        serde_json::from_slice(&read(&pending_path(layout), 1024 * 1024)?)?;
    ensure!(
        pending.version == 1 && pending.operation == "apply-hosted-chains",
        "unknown pending configuration operation"
    );
    pending.topology.validate()?;
    Ok(pending.topology)
}
pub fn finish_topology(layout: &Layout, topology: &crate::topology::Topology) -> Result<()> {
    if !pending_path(layout).exists() {
        return Ok(());
    }
    let bytes = read(&pending_path(layout), 1024 * 1024)?;
    ensure!(
        serde_json::to_value(desired_topology(layout)?)? == serde_json::to_value(topology)?,
        "pending topology changed during apply"
    );
    write(
        &layout.root.join("lattice.json"),
        &serde_json::to_vec_pretty(topology)?,
        false,
    )?;
    remove_if_unchanged(&pending_path(layout), &bytes)?;
    File::open(layout.root.join("run"))?.sync_all()?;
    Ok(())
}

pub fn supervisor_lease(layout: &Layout) -> Result<File> {
    layout.ensure("run")?;
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(layout.root.join("run/supervisor.lock"))?;
    file.try_lock_exclusive()
        .map_err(|_| anyhow::anyhow!("a foreground supervisor owns this host; stop it first"))?;
    Ok(file)
}
