use anyhow::{Result, ensure};
use lattice_sdk::primitives::ChainRuntimeContext;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    io::Read,
    path::Path,
};
pub const NEXUS_GENESIS: &str = "bafyreiggtg4ezifboyekbf4gxst2jr3mpjqcxsbmopy7w6fp46g4ngpgxa";
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(untagged)]
pub enum ChainPeers {
    Nexus(Vec<String>),
    ByChain(BTreeMap<String, Vec<String>>),
}
impl ChainPeers {
    pub fn by_chain(&self) -> BTreeMap<String, Vec<String>> {
        match self {
            Self::Nexus(peers) => BTreeMap::from([("Nexus".into(), peers.clone())]),
            Self::ByChain(peers) => peers.clone(),
        }
    }
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Topology {
    pub listen: u16,
    pub rpc: u16,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub web_socket: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub peers: Option<ChainPeers>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub public_read: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub external_address: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub public_read_rate: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub public_read_expensive_rate: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub public_read_max_rate: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub public_read_fetches: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "publicReadURL")]
    pub public_read_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub public_submit: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub public_submit_rate: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min_relay_fee: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub serving_max_concurrent: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub serving_max_in_flight_volume_bytes: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_acquisition_bytes: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub block_bodies: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub finality_window: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub finality_windows: Option<BTreeMap<String, u64>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub announce_lifetime: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hosted_chains: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mine: Option<TopologyMine>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rpc_allowed_origins: Option<Vec<String>>,
}
impl Default for Topology {
    fn default() -> Self {
        Self {
            listen: 4001,
            rpc: 8080,
            web_socket: None,
            peers: None,
            public_read: None,
            external_address: None,
            public_read_rate: None,
            public_read_expensive_rate: None,
            public_read_max_rate: None,
            public_read_fetches: None,
            public_read_url: None,
            public_submit: None,
            public_submit_rate: None,
            min_relay_fee: None,
            serving_max_concurrent: None,
            serving_max_in_flight_volume_bytes: None,
            max_acquisition_bytes: None,
            block_bodies: None,
            state: None,
            finality_window: None,
            finality_windows: None,
            announce_lifetime: None,
            hosted_chains: None,
            mine: None,
            rpc_allowed_origins: None,
        }
    }
}
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TopologyMine {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub worker: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub workers: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub batch_size: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recipients: Option<BTreeMap<String, String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min_work: Option<BTreeMap<String, String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min_block_interval_seconds: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub template_timeout_seconds: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub round_deadline_multiplier: Option<i64>,
}
pub fn valid_path(path: &str) -> bool {
    let parts: Vec<_> = path.split('/').map(str::to_owned).collect();
    let genesis = (parts.len() == 1).then_some(NEXUS_GENESIS);
    ChainRuntimeContext::new(parts, genesis).is_ok()
}
impl Topology {
    pub fn load(root: &Path) -> Result<Self> {
        let mut bytes = Vec::new();
        std::fs::File::open(root.join("lattice.json"))?
            .take(1024 * 1024 + 1)
            .read_to_end(&mut bytes)?;
        ensure!(bytes.len() <= 1024 * 1024, "topology exceeds 1 MiB");
        let value: Self = serde_json::from_slice(&bytes)?;
        value.validate()?;
        Ok(value)
    }
    pub fn validate(&self) -> Result<()> {
        let mut listed = BTreeSet::from(["Nexus".to_owned()]);
        for child in self.hosted_chains.as_deref().unwrap_or_default() {
            let parent = child.rsplit_once('/').map(|p| p.0);
            ensure!(
                valid_path(child)
                    && child != "Nexus"
                    && parent.is_some_and(|p| listed.contains(p))
                    && listed.insert(child.clone()),
                "hosted child must be new, canonical and listed after its parent"
            );
        }
        if let Some(peers) = &self.peers {
            ensure!(
                peers.by_chain().keys().all(|k| listed.contains(k)),
                "peers name an unhosted chain"
            );
        }
        ensure!(
            self.block_bodies
                .as_deref()
                .is_none_or(|m| ["all", "none"].contains(&m)),
            "blockBodies must be all or none"
        );
        ensure!(
            self.state
                .as_deref()
                .is_none_or(|m| ["all", "tip", "none"].contains(&m)),
            "state must be all, tip or none"
        );
        if let Some(windows) = &self.finality_windows {
            ensure!(
                windows.keys().all(|path| valid_path(
                    &path
                        .split('/')
                        .filter(|s| !s.is_empty())
                        .collect::<Vec<_>>()
                        .join("/")
                )),
                "finalityWindows contains an invalid path"
            );
        }
        ensure!(
            self.announce_lifetime
                .is_none_or(|n| (1..=86400).contains(&n)),
            "announceLifetime must be 1 through 86400"
        );
        let mut ports = BTreeSet::new();
        for port in [Some(self.listen), Some(self.rpc), self.public_read]
            .into_iter()
            .flatten()
        {
            ensure!(port > 0 && ports.insert(port), "port is zero or used twice");
        }
        if let Some(port) = self.web_socket.filter(|p| *p != 0) {
            ensure!(ports.insert(port), "webSocket port is used twice");
        }
        for rate in [
            self.public_read_rate,
            self.public_read_expensive_rate,
            self.public_read_max_rate,
            self.public_submit_rate,
        ]
        .into_iter()
        .flatten()
        {
            ensure!(
                rate.is_finite() && rate >= 0.,
                "public rates must be finite and nonnegative"
            );
        }
        if let Some(value) = &self.public_read_url {
            let url = url::Url::parse(value)?;
            ensure!(
                value.len() <= 512
                    && value.bytes().all(|b| (0x21..=0x7e).contains(&b))
                    && ["http", "https"].contains(&url.scheme())
                    && url.host_str().is_some_and(|h| !h.is_empty())
                    && url.username().is_empty()
                    && url.password().is_none()
                    && !value.contains(['?', '#']),
                "publicReadURL must be ASCII HTTP(S) without credentials, query or fragment"
            );
        }
        ensure!(
            self.max_acquisition_bytes
                .is_none_or(|n| n >= 64 * 1024 * 1024),
            "maxAcquisitionBytes is below one acquisition allowance"
        );
        ensure!(
            self.public_read_fetches.is_none_or(|n| n >= 0),
            "publicReadFetches must be nonnegative"
        );
        ensure!(
            self.public_submit != Some(true) || self.public_read.is_some(),
            "publicSubmit requires publicRead"
        );
        if let Some(mine) = &self.mine {
            ensure!(
                mine.workers.is_none_or(|n| n >= 1),
                "mine.workers must be positive"
            );
            ensure!(
                mine.batch_size.is_none_or(|n| n >= 1),
                "mine.batchSize must be positive"
            );
            ensure!(
                mine.template_timeout_seconds
                    .is_none_or(|n| (1..=60).contains(&n)),
                "mine.templateTimeoutSeconds must be 1 through 60"
            );
            ensure!(
                mine.round_deadline_multiplier.is_none_or(|n| n >= 1),
                "mine.roundDeadlineMultiplier must be positive"
            );
            for map in [&mine.recipients, &mine.min_work].into_iter().flatten() {
                ensure!(
                    map.keys().all(|p| listed.contains(p)),
                    "mining configuration names an unhosted chain"
                );
            }
        }
        Ok(())
    }
    pub fn hosted_paths(&self) -> Vec<String> {
        let mut paths = vec!["Nexus".into()];
        paths.extend(self.hosted_chains.clone().unwrap_or_default());
        paths
    }
    /// Refuse settings the current Rust daemon cannot honor, rather than dropping
    /// operator intent while translating the compatible topology model.
    pub fn ensure_daemon_supported(&self) -> Result<()> {
        ensure!(
            self.web_socket.is_none_or(|p| p == 0),
            "Rust daemon WebSocket listener is not implemented"
        );
        ensure!(
            self.external_address.is_none(),
            "Rust daemon externalAddress configuration is not implemented"
        );
        ensure!(
            self.serving_max_concurrent.is_none()
                && self.serving_max_in_flight_volume_bytes.is_none()
                && self.max_acquisition_bytes.is_none(),
            "Rust daemon serving/acquisition limit configuration is not implemented"
        );
        ensure!(
            self.finality_window.is_none_or(|n| n == 0)
                && self
                    .finality_windows
                    .as_ref()
                    .is_none_or(|m| m.values().all(|n| *n == 0)),
            "Rust daemon finality-window configuration is not implemented"
        );
        Ok(())
    }
}
