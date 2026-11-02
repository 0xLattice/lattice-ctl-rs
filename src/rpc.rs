use anyhow::{Context, Result, ensure};
use lattice_sdk::{ClientOptions, NodeClient, node_cookie_authorization};
use serde_json::Value;
use std::{path::PathBuf, time::Duration};
use tokio_util::sync::CancellationToken;
use url::{Host, Url};

pub struct LocalRpc {
    base: Url,
    cookie: PathBuf,
    chain: Vec<String>,
}
impl LocalRpc {
    pub fn new(base: &str, cookie: PathBuf, chain: &str) -> Result<Self> {
        let base = lattice_sdk::normalize_node_url(base)?;
        ensure!(
            matches!(base.host(), Some(Host::Ipv4(ip)) if ip.is_loopback())
                || matches!(base.host(), Some(Host::Ipv6(ip)) if ip.is_loopback()),
            "operator RPC requires a literal loopback IP"
        );
        ensure!(
            base.path() == "/",
            "operator RPC URL must not contain a path"
        );
        Ok(Self {
            base,
            cookie,
            chain: crate::transaction::chain_path(chain)?,
        })
    }
    // Re-read per request: the daemon rotates its cookie on restart.
    pub fn options(&self) -> Result<ClientOptions> {
        let cookie = std::fs::read_to_string(&self.cookie).context("reading node cookie")?;
        Ok(ClientOptions {
            timeout: Duration::from_secs(30),
            authorization: Some(node_cookie_authorization(&cookie)?),
            ..Default::default()
        })
    }
    pub fn client(&self) -> Result<NodeClient> {
        Ok(NodeClient::new(
            self.base.as_str(),
            self.chain.clone(),
            self.options()?,
        )?)
    }
    pub fn transaction_endpoint(&self) -> Result<String> {
        Ok(self.base.join("transactions")?.to_string())
    }
    pub async fn request(
        &self,
        method: reqwest::Method,
        path: &str,
        body: Option<Value>,
    ) -> Result<Value> {
        ensure!(
            path.starts_with('/')
                && !path.starts_with("//")
                && !path.contains(['?', '#'])
                && !path.split('/').any(|s| s == ".." || s == "."),
            "RPC path must be absolute without query, fragment or traversal"
        );
        let mut url = self.base.clone();
        url.set_path(path);
        // Host-wide routes reject query strings; child author POST bodies carry
        // their own chainPath. Nexus reads use the endpoint's root default.
        let host_wide = matches!(
            path,
            "/hosted-chains" | "/pins" | "/sweep" | "/child-blocks" | "/child-blocks/build"
        );
        if self.chain != ["Nexus"] && !host_wide {
            url.query_pairs_mut()
                .append_pair("chainPath", &self.chain.join("/"));
        }
        let options = self.options()?;
        let http = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(options.timeout)
            .build()?;
        let mut request = http
            .request(method, url)
            .header("Authorization", options.authorization.unwrap())
            .header("Cache-Control", "no-cache");
        if let Some(body) = body {
            let bytes = serde_json::to_vec(&body)?;
            ensure!(bytes.len() <= 1024 * 1024, "request exceeds 1 MiB");
            request = request
                .header("Content-Type", "application/json")
                .body(bytes);
        }
        let mut response = request.send().await?;
        let status = response.status();
        let mut bytes = vec![];
        while let Some(chunk) = response.chunk().await? {
            ensure!(
                chunk.len() <= options.maximum_response_bytes.saturating_sub(bytes.len()),
                "RPC response exceeds limit"
            );
            bytes.extend_from_slice(&chunk);
        }
        ensure!(
            status.is_success(),
            "RPC HTTP {}: {}",
            status.as_u16(),
            String::from_utf8_lossy(&bytes[..bytes.len().min(512)])
        );
        serde_json::from_slice(&bytes).context("RPC did not return JSON")
    }
    pub async fn nonce(&self, address: &str) -> Result<u64> {
        let cancel = CancellationToken::new();
        // Never infer zero from 404: pruned/unavailable state can also return it.
        let account = self.client()?.account(address, &cancel).await.context(
            "account unavailable; provide --nonce only after independently checking state",
        )?;
        let nonce = account.nonce.0.to_string().parse()?;
        let pool = self.client()?.mempool(&cancel).await?;
        for cid in pool.transactions {
            match self.client()?.transaction(&cid, &cancel).await {
                Ok(entry) if entry.signers.first().is_some_and(|s| s == address) => anyhow::bail!(
                    "signer has pending transaction at nonce {}; wait until mined or explicitly replace with --nonce and higher --fee",
                    entry.nonce.0
                ),
                Ok(_) | Err(lattice_sdk::ClientError::Node { status: 404, .. }) => {}
                Err(error) => return Err(error.into()),
            }
        }
        Ok(nonce)
    }
    pub async fn fee(&self) -> Result<u64> {
        Ok(self
            .client()?
            .fees(&CancellationToken::new())
            .await?
            .min_relay_fee
            .0
            .to_string()
            .parse()?)
    }
}
