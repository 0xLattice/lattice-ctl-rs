use anyhow::{Context, Result};
use clap::{Args, Parser, Subcommand};
use lattice_ctl::{
    keys,
    rpc::LocalRpc,
    transaction::{self, ActionPlan},
};
use lattice_sdk::submission::HttpTransactionSubmitter;
use serde_json::{Value, json};
use std::path::PathBuf;
use tokio_util::sync::CancellationToken;

#[derive(Parser)]
#[command(
    name = "lattice",
    about = "Local Lattice platform keys, RPC and transactions"
)]
struct Cli {
    #[arg(long, global = true, default_value = ".")]
    root: PathBuf,
    #[arg(long, global = true)]
    rpc: Option<String>,
    #[arg(long, global = true)]
    cookie: Option<PathBuf>,
    #[arg(long, global = true, default_value = "Nexus")]
    chain: String,
    #[command(subcommand)]
    command: Command,
}
#[derive(Subcommand)]
enum Command {
    /// Initialize local topology and a private overlay identity.
    Init {
        #[arg(long, default_value_t = 4001)]
        listen: u16,
        #[arg(long, default_value_t = 8080)]
        rpc_port: u16,
        #[arg(long)]
        no_default_peers: bool,
    },
    Identity,
    Up {
        #[arg(long)]
        node_binary: PathBuf,
        #[arg(long, default_value = "127.0.0.1")]
        listen_host: std::net::IpAddr,
    },
    Down,
    Topology,

    Key {
        #[command(subcommand)]
        command: KeyCommand,
    },
    Status,
    Rpc {
        path: String,
        #[arg(long, default_value="GET", value_parser=["GET","POST","DELETE"])]
        method: String,
        #[arg(long)]
        body: Option<PathBuf>,
    },
    Tx {
        #[command(flatten)]
        options: TxOptions,
        #[command(subcommand)]
        command: TxCommand,
    },
}
#[derive(Subcommand)]
enum KeyCommand {
    Generate {
        #[arg(long)]
        out: PathBuf,
    },
    Show {
        #[arg(long)]
        key: PathBuf,
    },
}
#[derive(Args)]
struct TxOptions {
    #[arg(long, global = true)]
    key: Option<PathBuf>,
    #[arg(long, global = true)]
    nonce: Option<u64>,
    #[arg(long, global = true)]
    fee: Option<u64>,
    #[arg(long, global = true)]
    dry_run: bool,
}
#[derive(Subcommand)]
enum TxCommand {
    Send {
        #[arg(long)]
        to: String,
        #[arg(long)]
        amount: u64,
    },
    Deposit {
        #[arg(long)]
        swap_nonce: u64,
        #[arg(long)]
        demand: u64,
        #[arg(long)]
        lock: u64,
    },
    Receipt {
        #[arg(long)]
        swap_nonce: u64,
        #[arg(long)]
        demand: u64,
        #[arg(long)]
        demander: String,
        #[arg(long)]
        directory: String,
    },
    Withdraw {
        #[arg(long)]
        swap_nonce: u64,
        #[arg(long)]
        demand: u64,
        #[arg(long)]
        demander: String,
        #[arg(long)]
        amount: u64,
        #[arg(long)]
        to: Option<String>,
    },
    CommitChild {
        #[arg(long)]
        directory: String,
        #[arg(long)]
        block: String,
        #[arg(long)]
        parent_state: String,
        #[arg(long)]
        previous: Option<String>,
    },
}
impl From<TxCommand> for ActionPlan {
    fn from(value: TxCommand) -> Self {
        match value {
            TxCommand::Send { to, amount } => Self::Send { to, amount },
            TxCommand::Deposit {
                swap_nonce,
                demand,
                lock,
            } => Self::Deposit {
                nonce: swap_nonce,
                demand,
                lock,
            },
            TxCommand::Receipt {
                swap_nonce,
                demand,
                demander,
                directory,
            } => Self::Receipt {
                nonce: swap_nonce,
                demand,
                demander,
                directory,
            },
            TxCommand::Withdraw {
                swap_nonce,
                demand,
                demander,
                amount,
                to,
            } => Self::Withdraw {
                nonce: swap_nonce,
                demand,
                demander,
                amount,
                to,
            },
            TxCommand::CommitChild {
                directory,
                block,
                parent_state,
                previous,
            } => Self::CommitChild {
                directory,
                block,
                parent_state,
                previous,
            },
        }
    }
}
fn output(value: &impl serde::Serialize) -> Result<()> {
    println!("{}", serde_json::to_string_pretty(value)?);
    Ok(())
}
#[tokio::main]
async fn main() {
    if let Err(error) = run(Cli::parse()).await {
        eprintln!("error: {error:#}");
        std::process::exit(1);
    }
}
async fn run(cli: Cli) -> Result<()> {
    match &cli.command {
        Command::Init {
            listen,
            rpc_port,
            no_default_peers,
        } => {
            return output(&lattice_ctl::lifecycle::init(
                &cli.root,
                *listen,
                *rpc_port,
                *no_default_peers,
            )?);
        }
        Command::Identity => return output(&lattice_ctl::lifecycle::identity(&cli.root)?),
        Command::Up {
            node_binary,
            listen_host,
        } => {
            return output(
                &lattice_ctl::lifecycle::up(&cli.root, node_binary, *listen_host).await?,
            );
        }
        Command::Down => return output(&lattice_ctl::lifecycle::down(&cli.root).await?),
        Command::Topology => {
            return output(&serde_json::to_value(
                lattice_ctl::topology::Topology::load(&cli.root)?,
            )?);
        }
        _ => {}
    }
    if let Command::Key { command } = cli.command {
        return match command {
            KeyCommand::Generate { out } => output(&keys::generate(&out)?),
            KeyCommand::Show { key } => output(&keys::PublicKeyInfo::from(&keys::load(&key)?)),
        };
    }
    let base = match cli.rpc {
        Some(base) => base,
        None => {
            let topology = cli.root.join("lattice.json");
            let port = if topology.exists() {
                let value: Value = serde_json::from_slice(&std::fs::read(topology)?)?;
                let n = value
                    .get("rpc")
                    .and_then(Value::as_u64)
                    .context("lattice.json requires rpc port")?;
                u16::try_from(n).context("rpc port out of range")?
            } else {
                8080
            };
            format!("http://127.0.0.1:{port}")
        }
    };
    let rpc = LocalRpc::new(
        &base,
        cli.cookie
            .unwrap_or_else(|| cli.root.join("chains/Nexus/.cookie")),
        &cli.chain,
    )?;
    match cli.command {
        Command::Key { .. }
        | Command::Init { .. }
        | Command::Identity
        | Command::Up { .. }
        | Command::Down
        | Command::Topology => unreachable!(),
        Command::Status => output(&rpc.request(reqwest::Method::GET, "/status", None).await?),
        Command::Rpc { path, method, body } => {
            let body = body
                .map(|path| -> Result<Value> {
                    let bytes = std::fs::read(path)?;
                    anyhow::ensure!(bytes.len() <= 1024 * 1024, "body file too large");
                    Ok(serde_json::from_slice(&bytes)?)
                })
                .transpose()?;
            output(&rpc.request(method.parse()?, &path, body).await?)
        }
        Command::Tx { options, command } => {
            let signer = keys::load(
                options
                    .key
                    .as_deref()
                    .context("--key is required for transactions")?,
            )?;
            let nonce = match options.nonce {
                Some(n) => n,
                None => rpc.nonce(signer.address()).await?,
            };
            let fee = match options.fee {
                Some(n) => n,
                None => rpc.fee().await?,
            };
            let body = transaction::compose(&signer, command.into(), &cli.chain, nonce, fee)?;
            let (signatures, payload) = transaction::signed(&signer, &body)?;
            if options.dry_run {
                output(&payload)
            } else {
                let submitted = HttpTransactionSubmitter::new(
                    &rpc.transaction_endpoint()?,
                    Some(rpc.options()?),
                )?
                .submit_signed(&signatures, &body, &CancellationToken::new())
                .await?;
                output(
                    &json!({"transactionCID":submitted.transaction_cid,"mempoolCount":submitted.mempool_count,"chainPath":body.chain_path,"signer":signer.address(),"nonce":nonce.to_string()}),
                )
            }
        }
    }
}
