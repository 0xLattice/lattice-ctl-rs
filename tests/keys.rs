#![cfg(unix)]
use lattice_ctl::keys;
use std::{
    fs,
    os::unix::fs::{PermissionsExt, symlink},
};
#[test]
fn keys_are_private_exclusive_bound_and_never_follow_symlinks() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("wallet.json");
    let public = keys::generate(&path).unwrap();
    assert_eq!(
        fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o600
    );
    let original = fs::read(&path).unwrap();
    assert!(keys::generate(&path).is_err());
    assert_eq!(fs::read(&path).unwrap(), original);
    let account = keys::load(&path).unwrap();
    assert_eq!(account.address(), public.address);
    assert_eq!(account.public_key(), public.public_key);
    let link = temp.path().join("link");
    symlink(&path, &link).unwrap();
    assert!(keys::load(&link).is_err());
    assert!(keys::generate(&link).is_err());
    fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
    assert!(keys::load(&path).is_err());
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
    let mut value: serde_json::Value = serde_json::from_slice(&original).unwrap();
    value["publicKey"] = "ed0100".into();
    fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
    assert!(keys::load(&path).is_err());
}
#[test]
fn cli_public_output_does_not_contain_private_material() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("wallet.json");
    let result = std::process::Command::new(env!("CARGO_BIN_EXE_lattice"))
        .args(["key", "generate", "--out", path.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let stored: serde_json::Value = serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
    let public: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
    assert!(public.get("privateKey").is_none());
    assert_eq!(public["address"], stored["address"]);
    assert!(
        !String::from_utf8_lossy(&result.stdout).contains(stored["privateKey"].as_str().unwrap())
    );
}
