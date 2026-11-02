//! Local operator CLI building blocks. Signing does not establish admission.
pub mod keys;
pub mod rpc;
pub mod transaction;

pub mod bootstrap;
#[cfg(unix)]
pub mod lifecycle;
pub mod topology;
#[cfg(not(unix))]
pub mod lifecycle {
    use anyhow::{Result, bail};
    use serde_json::Value;
    use std::{net::IpAddr, path::Path};
    pub fn init(_: &Path, _: u16, _: u16, _: bool) -> Result<Value> {
        bail!("host lifecycle requires Linux or macOS")
    }
    pub fn identity(_: &Path) -> Result<Value> {
        bail!("host lifecycle requires Linux or macOS")
    }
    pub async fn up(_: &Path, _: &Path, _: IpAddr) -> Result<Value> {
        bail!("host lifecycle requires Linux or macOS")
    }
    pub async fn down(_: &Path) -> Result<Value> {
        bail!("host lifecycle requires Linux or macOS")
    }
}
