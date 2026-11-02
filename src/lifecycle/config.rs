use super::files::{self, Layout};
use crate::topology::{NEXUS_GENESIS, Topology};
use anyhow::{Result, ensure};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::net::IpAddr;
pub(crate) fn encoded(path: &str) -> String {
    let mut result = String::new();
    for b in path.bytes() {
        if b.is_ascii_alphanumeric() || b"._-".contains(&b) {
            result.push(b as char)
        } else {
            result.push_str(&format!("%{b:02X}"))
        }
    }
    result
}
pub(super) fn peer(value: &str) -> Result<Value> {
    let (key, address) = value
        .split_once('@')
        .ok_or_else(|| anyhow::anyhow!("peer must be publicKey@host:port"))?;
    ensure!(
        key.len() == 64 && hex::decode(key)?.len() == 32,
        "peer public key must be 32 bytes"
    );
    let (host, port) = address
        .rsplit_once(':')
        .ok_or_else(|| anyhow::anyhow!("peer has no port"))?;
    let host = host
        .strip_prefix('[')
        .and_then(|h| h.strip_suffix(']'))
        .unwrap_or(host);
    let port: u16 = port.parse()?;
    ensure!(
        port > 0
            && !host.is_empty()
            && !host
                .chars()
                .any(|c| c.is_whitespace() || c == '/' || c == '@'),
        "invalid peer endpoint"
    );
    Ok(json!({"publicKey":key.to_ascii_lowercase(),"host":host,"port":port}))
}
fn defaults(fixed: bool) -> Vec<Value> {
    let keys = [
        "139b8f3639e7c515417c63bd3a652a5c6fd4a1a2d0baed8e33ea63047995fe64",
        "35edf67bfe3d612aeb1f0e25da9d3f0ced44dbf79d34f00c548cf9005be6eb7d",
        "9cace839489acb30385a9f20025cb9d6365283c81dce14cadab26507065acd4e",
        "57f80deb3b00da1b14b630638a4d0307be98126ec1d550476e4889087bb22d0f",
    ];
    let hosts = if fixed {
        [
            "137.66.36.149",
            "137.66.14.188",
            "137.66.16.78",
            "169.155.61.57",
        ]
    } else {
        [
            "seed1.lattice.build",
            "seed2.lattice.build",
            "seed3.lattice.build",
            "seed4.lattice.build",
        ]
    };
    keys.into_iter()
        .zip(hosts)
        .map(|(key, host)| json!({"publicKey":key,"host":host,"port":4001}))
        .collect()
}
pub fn hosted(layout: &Layout, topology: &Topology) -> Result<(Value, BTreeMap<String, String>)> {
    let peers = topology
        .peers
        .as_ref()
        .map(|p| p.by_chain())
        .unwrap_or_default();
    let (mut hosts, mut premine, mut genesis, mut modules, mut chain_peers) =
        (vec![], vec![], vec![], vec![], vec![]);
    let mut inputs = BTreeMap::new();
    for path in topology.hosted_chains.as_deref().unwrap_or_default() {
        let name = encoded(path);
        let spec = layout.root.join(format!("specs/{name}.json"));
        let record =
            |file: &std::path::Path, inputs: &mut BTreeMap<String, String>| -> Result<()> {
                let bytes = files::read(file, 4 * 1024 * 1024)?;
                inputs.insert(
                    file.to_string_lossy().into_owned(),
                    lattice_sdk::primitives::crypto::sha256(&hex::encode(bytes)),
                );
                Ok(())
            };
        if spec.exists() {
            record(&spec, &mut inputs)?;
            hosts.push(format!("{path}={}", spec.display()));
            let recipient = layout.root.join(format!("specs/{name}.premine-to"));
            if recipient.exists() {
                record(&recipient, &mut inputs)?;
                let bytes = files::read(&recipient, 4096)?;
                let address = std::str::from_utf8(&bytes)?.trim();
                if !address.is_empty() {
                    premine.push(format!("{path}={address}"));
                }
            }
            let transactions = layout.root.join(format!("specs/{name}.genesis.json"));
            if transactions.exists() {
                record(&transactions, &mut inputs)?;
                genesis.push(format!("{path}={}", transactions.display()));
            }
        } else {
            hosts.push(path.clone());
        }
        let directory = layout.root.join(format!("specs/{name}.policies"));
        if directory.exists() {
            ensure!(
                !directory.is_symlink(),
                "policy directory cannot be a symlink"
            );
            let mut files: Vec<_> = std::fs::read_dir(directory)?
                .map(|e| e.map(|e| e.path()))
                .collect::<std::io::Result<_>>()?;
            files.sort();
            for file in files {
                record(&file, &mut inputs)?;
                modules.push(file.to_string_lossy().into_owned());
            }
        }
        for endpoint in peers.get(path).into_iter().flatten() {
            peer(endpoint)?;
            chain_peers.push(format!("{path}={endpoint}"));
        }
    }
    Ok((
        json!({"hostChain":hosts,"premineTo":premine,"genesisTransactions":genesis,"policyModule":modules,"chainPeer":chain_peers}),
        inputs,
    ))
}
pub fn build(layout: &Layout, topology: &Topology, listen_host: IpAddr) -> Result<(Value, String)> {
    topology.validate()?;
    topology.ensure_daemon_supported()?;
    let (hosted, inputs) = hosted(layout, topology)?;
    let peers = topology
        .peers
        .as_ref()
        .map(|p| p.by_chain())
        .unwrap_or_default();
    let configured = peers.get("Nexus");
    ensure!(
        configured.is_some() || std::env::var("LATTICE_FORBID_DEFAULT_PEERS").as_deref() != Ok("1"),
        "default public peers are forbidden in this environment; configure peers explicitly"
    );
    let seeds = configured
        .map(|rows| rows.iter().map(|p| peer(p)).collect::<Result<Vec<_>>>())
        .transpose()?
        .unwrap_or_default();
    let mut value = json!({"directory":layout.root.join("chains/Nexus"),"identityKey":layout.identity(),"genesisCid":NEXUS_GENESIS,"bootstrapFiles":[layout.root.join("genesis.json")],
        "listen":std::net::SocketAddr::new(listen_host,topology.listen).to_string(),"rpc":format!("127.0.0.1:{}",topology.rpc),"cookieFile":layout.cookie(),"seeds":seeds,"seedPeers":if configured.is_none(){defaults(false)}else{vec![]},"fixedSeedPeers":if configured.is_none(){defaults(true)}else{vec![]},"allowedOrigins":topology.rpc_allowed_origins.clone().unwrap_or_default()});
    for (k, v) in hosted.as_object().unwrap() {
        value[k] = v.clone();
    }
    for (k, v) in [
        ("state", topology.state.as_ref().map(|s| json!(s))),
        (
            "blockBodies",
            topology.block_bodies.as_ref().map(|s| json!(s)),
        ),
        ("minRelayFee", topology.min_relay_fee.map(|n| json!(n))),
        (
            "announceLifetime",
            topology.announce_lifetime.map(|n| json!(n)),
        ),
        (
            "publicRead",
            topology.public_read.map(|p| json!(format!("0.0.0.0:{p}"))),
        ),
        (
            "publicReadURL",
            topology.public_read_url.as_ref().map(|s| json!(s)),
        ),
        ("publicSubmit", topology.public_submit.map(|n| json!(n))),
        (
            "publicReadFetches",
            topology.public_read_fetches.map(|n| json!(n)),
        ),
    ] {
        if let Some(v) = v {
            value[k] = v;
        }
    }
    let mut rates = serde_json::Map::new();
    for (k, v) in [
        ("general", topology.public_read_rate),
        ("expensive", topology.public_read_expensive_rate),
        ("listener", topology.public_read_max_rate),
        ("submit", topology.public_submit_rate),
    ] {
        if let Some(v) = v {
            rates.insert(k.into(), json!(v));
        }
    }
    if !rates.is_empty() {
        value["publicReadRates"] = Value::Object(rates);
    }
    let fingerprint = lattice_sdk::primitives::crypto::sha256(&serde_json::to_string(
        &json!({"config":value,"inputs":inputs}),
    )?);
    Ok((value, fingerprint))
}
