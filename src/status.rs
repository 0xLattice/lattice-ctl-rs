//! Tree-wide operator health reads. These snapshots establish no consensus proof.
use crate::{
    lifecycle::{
        files::{self, Layout},
        process,
    },
    topology::Topology,
};
use anyhow::{Result, ensure};
use lattice_sdk::{ClientOptions, NodeClient};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{path::Path, sync::Arc, time::Duration};
use tokio_util::sync::CancellationToken;

#[derive(Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ProcessState {
    Down,
    Running,
    Unverified,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Row {
    pub chain_path: Vec<String>,
    pub state: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub health: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TreeStatus {
    pub process: ProcessState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ownership_error: Option<String>,
    pub pending_configuration: bool,
    pub chains: Vec<Row>,
}
#[derive(Deserialize)]
struct RecordedProcess {
    process: process::ProcessIdentity,
}
fn ownership(layout: &Layout) -> Result<bool> {
    if !layout.pid().exists() {
        return Ok(false);
    }
    let record: RecordedProcess =
        serde_json::from_slice(&files::read(&layout.pid(), 1024 * 1024)?)?;
    process::matches(&record.process)
}
pub async fn read(root: &Path, endpoint: Option<&str>) -> Result<TreeStatus> {
    let layout = Layout::open(root)?;
    // Report committed topology; pending intent is identified separately, not
    // silently presented as already applied to the running node.
    let topology = Topology::load(&layout.root)?;
    let base = endpoint
        .map(str::to_owned)
        .unwrap_or_else(|| format!("http://127.0.0.1:{}", topology.rpc));
    // Reuse the operator URL policy, without reading its cookie.
    crate::rpc::LocalRpc::new(&base, layout.cookie(), "Nexus")?;
    let (process, ownership_error) = match ownership(&layout) {
        Ok(true) => (ProcessState::Running, None),
        Ok(false) => (ProcessState::Down, None),
        Err(error) => (ProcessState::Unverified, Some(error.to_string())),
    };
    let mut report = TreeStatus {
        process,
        ownership_error,
        pending_configuration: files::pending_path(&layout).exists(),
        chains: topology
            .hosted_paths()
            .into_iter()
            .map(|path| Row {
                chain_path: path.split('/').map(str::to_owned).collect(),
                state: if process == ProcessState::Down {
                    "down"
                } else {
                    "unreachable"
                }
                .into(),
                health: None,
                error: None,
            })
            .collect(),
    };
    if process == ProcessState::Down {
        return Ok(report);
    }
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    let permits = Arc::new(tokio::sync::Semaphore::new(16));
    let mut tasks = tokio::task::JoinSet::new();
    for (index, row) in report.chains.iter().enumerate() {
        let path = row.chain_path.clone();
        let base = base.clone();
        let permits = permits.clone();
        tasks.spawn(async move {
            let result=async {
                let _permit=permits.acquire_owned().await?;
                let client=NodeClient::new(&base,path.clone(),ClientOptions{timeout:Duration::from_secs(5),maximum_response_bytes:64*1024,authorization:None})?;
                let health=client.health(&CancellationToken::new()).await?;
                ensure!(health.chain_path==path,"health chain path differs from request");
                Ok::<_,anyhow::Error>(json!({"phase":health.phase,"chainPath":health.chain_path,"nexusGenesisCID":health.nexus_genesis_cid,"height":health.height,"tipCID":health.tip_cid,"bestHeaderHeight":health.best_header_height,"mempoolCount":health.mempool_count,"mempoolBytes":health.mempool_bytes}))
            }.await;
            (index,result)
        });
    }
    while !tasks.is_empty() {
        let next = tokio::select! {biased;_=tokio::time::sleep_until(deadline)=>break,next=tasks.join_next()=>next};
        if let Some(Ok((index, result))) = next {
            match result {
                Ok(health) => {
                    report.chains[index].state = "available".into();
                    report.chains[index].health = Some(health);
                }
                Err(error) => report.chains[index].error = Some(error.to_string()),
            }
        }
    }
    tasks.abort_all();
    for row in &mut report.chains {
        if row.state == "unreachable" && row.error.is_none() {
            row.error = Some("health deadline exceeded".into());
        }
    }
    Ok(report)
}
fn safe(value: &str) -> String {
    value
        .chars()
        .map(|c| if c.is_control() { '?' } else { c })
        .collect()
}
pub fn table(report: &TreeStatus) -> String {
    let mut output = String::new();
    if report.pending_configuration {
        output.push_str("configuration: pending child update (showing committed topology)\n");
    }
    if report.process == ProcessState::Unverified {
        output.push_str("process: ownership unverified\n");
    }
    for row in &report.chains {
        let path = safe(&row.chain_path.join("/"));
        if row.state == "down" {
            output.push_str(&format!("{path}: down\n"));
            continue;
        }
        let Some(health) = &row.health else {
            output.push_str(&format!(
                "{path}: {}rpc unreachable\n",
                if report.process == ProcessState::Running {
                    "running, "
                } else {
                    ""
                }
            ));
            continue;
        };
        let phase = safe(health["phase"].as_str().unwrap_or("?"));
        let height = health["height"].as_str().unwrap_or("-");
        let tip = safe(
            &health["tipCID"]
                .as_str()
                .unwrap_or("-")
                .chars()
                .take(20)
                .collect::<String>(),
        );
        let pool = health["mempoolCount"].as_i64().unwrap_or(0);
        output.push_str(&format!(
            "{path}: {phase} height={height} tip={tip}… mempool={pool}\n"
        ));
    }
    output
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn table_preserves_wide_heights_and_escapes_controls() {
        let report = TreeStatus {
            process: ProcessState::Running,
            ownership_error: None,
            pending_configuration: false,
            chains: vec![Row {
                chain_path: vec!["Nexus".into()],
                state: "available".into(),
                health: Some(
                    json!({"phase":"active\u{1b}","height":"18446744073709551615","tipCID":"abc","mempoolCount":1}),
                ),
                error: None,
            }],
        };
        let text = table(&report);
        assert!(text.contains("18446744073709551615"));
        assert!(!text.contains('\u{1b}'));
    }
}
