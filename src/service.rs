//! Generate service definitions only. This module never invokes a service manager.
use anyhow::{Result, ensure};
use std::{net::IpAddr, path::Path};
fn quoted(path: &Path) -> Result<String> {
    ensure!(path.is_absolute(), "service paths must be absolute");
    let value = path
        .to_str()
        .ok_or_else(|| anyhow::anyhow!("service paths must be UTF-8"))?;
    ensure!(
        !value.chars().any(char::is_control),
        "service paths cannot contain control characters"
    );
    Ok(format!(
        "\"{}\"",
        value
            .replace('\\', "\\\\")
            .replace('"', "\\\"")
            .replace('%', "%%")
    ))
}
pub fn emit(root: &Path, binary: &Path, node_binary: &Path, listen_host: IpAddr) -> Result<String> {
    let layout = crate::lifecycle::files::Layout::open(root)?;
    let topology = crate::lifecycle::files::desired_topology(&layout)?;
    topology.validate()?;
    topology.ensure_daemon_supported()?;
    let root = quoted(&layout.root)?;
    let binary = quoted(binary)?;
    let node = quoted(node_binary)?;
    let mut result = format!(
        "# lattice-tree.service\n[Unit]\nDescription=Lattice hosted chain tree ({} chains)\nAfter=network-online.target\nWants=network-online.target\n\n[Service]\nType=simple\nExecStart=:{binary} --root {root} up --foreground --node-binary {node} --listen-host {listen_host}\nRestart=on-failure\nRestartSec=2\nKillMode=mixed\nTimeoutStopSec=90\n\n[Install]\nWantedBy=multi-user.target\n",
        topology.hosted_paths().len()
    );
    if topology.mine.is_some() {
        result.push_str(&format!("\n# lattice-mine.service\n[Unit]\nDescription=Lattice mining loop\nAfter=lattice-tree.service\nBindsTo=lattice-tree.service\n\n[Service]\nType=simple\nExecStart=:{binary} --root {root} mine run\nRestart=on-failure\nRestartSec=2\nKillMode=mixed\nTimeoutStopSec=90\n\n[Install]\nWantedBy=multi-user.target\n"));
    }
    Ok(result)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn paths_are_quoted_and_expansion_escaped() {
        assert_eq!(
            quoted(Path::new("/opt/a b/$x%q\\\"z")).unwrap(),
            "\"/opt/a b/$x%%q\\\\\\\"z\""
        );
        assert!(quoted(Path::new("/opt/a\nExecStart=bad")).is_err());
        assert!(quoted(Path::new("relative")).is_err());
    }
}
