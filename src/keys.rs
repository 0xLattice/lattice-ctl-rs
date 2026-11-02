use anyhow::Result;
#[cfg(unix)]
use anyhow::{Context, ensure};
use lattice_sdk::transactions::SigningAccount;
#[cfg(unix)]
use rand_core::RngCore;
#[cfg(unix)]
use serde::Deserialize;
use serde::Serialize;
use std::path::Path;
#[cfg(unix)]
use std::{
    fs::OpenOptions,
    io::{Read, Write},
};

#[cfg(unix)]
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct KeyFile {
    address: String,
    private_key: String,
    public_key: String,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PublicKeyInfo {
    pub address: String,
    pub public_key: String,
}
impl From<&SigningAccount> for PublicKeyInfo {
    fn from(account: &SigningAccount) -> Self {
        Self {
            address: account.address().into(),
            public_key: account.public_key().into(),
        }
    }
}
#[cfg(unix)]
pub fn generate(path: &Path) -> Result<PublicKeyInfo> {
    use std::os::unix::fs::OpenOptionsExt;
    let mut seed = [0; 32];
    rand_core::OsRng.fill_bytes(&mut seed);
    let account = SigningAccount::from_private_key(&seed)?;
    let file = KeyFile {
        address: account.address().into(),
        private_key: hex::encode(seed),
        public_key: account.public_key().into(),
    };
    let bytes = serde_json::to_vec_pretty(&file)?;
    let mut output = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
        .context("creating private key file (existing files are never overwritten)")?;
    output.write_all(&bytes)?;
    output.write_all(b"\n")?;
    output.sync_all()?;
    Ok(PublicKeyInfo::from(&account))
}
#[cfg(not(unix))]
pub fn generate(_: &Path) -> Result<PublicKeyInfo> {
    anyhow::bail!(
        "private key creation requires Unix permissions; Windows ACL support is not implemented"
    )
}

#[cfg(unix)]
pub fn load(path: &Path) -> Result<SigningAccount> {
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)
        .context("opening private key file")?;
    let meta = file.metadata()?;
    ensure!(meta.is_file(), "key must be a regular file");
    ensure!(
        meta.permissions().mode() & 0o077 == 0,
        "key file is group/other-readable; chmod 600 it"
    );
    let mut bytes = Vec::new();
    file.take(16 * 1024 + 1).read_to_end(&mut bytes)?;
    ensure!(bytes.len() <= 16 * 1024, "key file is too large");
    let decoded: KeyFile = serde_json::from_slice(&bytes).context("invalid key file JSON")?;
    let seed: [u8; 32] = hex::decode(&decoded.private_key)
        .context("privateKey must be hex")?
        .try_into()
        .map_err(|_| anyhow::anyhow!("privateKey must contain 32 bytes"))?;
    let account = SigningAccount::from_private_key(&seed)?;
    ensure!(
        account.public_key() == decoded.public_key,
        "publicKey does not match privateKey"
    );
    ensure!(
        account.address() == decoded.address,
        "address does not match publicKey"
    );
    Ok(account)
}
#[cfg(not(unix))]
pub fn load(_: &Path) -> Result<SigningAccount> {
    anyhow::bail!(
        "private key loading requires Unix permissions; Windows ACL support is not implemented"
    )
}
