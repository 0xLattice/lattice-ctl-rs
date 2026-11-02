#![cfg(unix)]
use serde_json::{Value, json};
use std::{path::Path, process::Command};

fn draft(root: &Path, key: &Path, chain: &str, to: &str) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_lattice"))
        .arg("--root")
        .arg(root)
        .args(["--chain", chain, "tx", "--key"])
        .arg(key)
        .args([
            "--nonce",
            "0",
            "--fee",
            "0",
            "--dry-run",
            "send",
            "--to",
            to,
            "--amount",
            "1",
        ])
        .output()
        .unwrap()
}

#[test]
fn managed_transactions_only_sign_for_committed_hosted_paths() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().join("managed");
    std::fs::create_dir(&root).unwrap();
    std::fs::write(
        root.join("lattice.json"),
        json!({"listen":4001,"rpc":8080,"hostedChains":["Nexus/Alpha"]}).to_string(),
    )
    .unwrap();
    let key = directory.path().join("key.json");
    let public = lattice_ctl::keys::generate(&key).unwrap();
    for chain in ["Nexus", "Nexus/Alpha"] {
        let result = draft(&root, &key, chain, &public.address);
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        let value: Value = serde_json::from_slice(&result.stdout).unwrap();
        assert_eq!(
            value["transaction"]["body"]["chainPath"],
            json!(chain.split('/').collect::<Vec<_>>())
        );
    }
    let rejected = draft(&root, &key, "Nexus/Beta", &public.address);
    assert!(!rejected.status.success());
    assert!(String::from_utf8_lossy(&rejected.stderr).contains("Nexus/Beta is not in the tree"));
    assert!(rejected.stdout.is_empty());

    // Standalone offline signing has no managed topology to consult.
    let standalone = directory.path().join("standalone");
    let result = draft(&standalone, &key, "Nexus/Beta", &public.address);
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(!standalone.exists());
}

#[test]
fn rpc_override_does_not_bypass_membership_or_touch_key_and_network() {
    let directory = tempfile::tempdir().unwrap();
    std::fs::write(
        directory.path().join("lattice.json"),
        json!({"listen":4001,"rpc":8080}).to_string(),
    )
    .unwrap();
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let missing_key = directory.path().join("missing-key.json");
    let output = Command::new(env!("CARGO_BIN_EXE_lattice"))
        .arg("--root")
        .arg(directory.path())
        .args([
            "--rpc",
            &endpoint,
            "--chain",
            "Nexus/Unknown",
            "tx",
            "--key",
        ])
        .arg(&missing_key)
        .args(["send", "--to", "unused", "--amount", "1"])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("Nexus/Unknown is not in the tree"));
    assert!(
        matches!(listener.accept(), Err(error) if error.kind() == std::io::ErrorKind::WouldBlock)
    );
    assert!(!missing_key.exists());
}
