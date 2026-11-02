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
    #[arg(long, global = true)]
    chain: Option<String>,
    #[command(subcommand)]
    command: Command,
}
#[derive(Subcommand)]
enum Command {
    #[cfg(unix)]
    Wipe,
    #[cfg(unix)]
    EmitSystemd {
        #[arg(long, default_value = "/usr/local/bin/lattice")]
        binary: PathBuf,
        #[arg(long, default_value = "/usr/local/bin/lattice-node")]
        node_binary: PathBuf,
        #[arg(long, default_value = "127.0.0.1")]
        listen_host: std::net::IpAddr,
    },
    #[cfg(unix)]
    Child {
        #[command(subcommand)]
        command: ChildCommand,
    },
    #[cfg(unix)]
    Mine {
        #[command(subcommand)]
        command: MineCommand,
    },
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
        #[cfg(unix)]
        #[arg(long)]
        foreground: bool,
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
    Status {
        #[arg(long)]
        json: bool,
    },
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

#[cfg(unix)]
#[derive(Subcommand)]
enum MineCommand {
    Start,
    Run {
        #[arg(long)]
        once: bool,
        #[arg(long, hide = true)]
        managed: bool,
    },
    Stop,
    Status,
}

#[cfg(unix)]
#[derive(Subcommand)]
enum ChildCommand {
    Host {
        path: String,
    },
    Apply,
    Create {
        path: String,
        #[arg(long)]
        spec: Option<PathBuf>,
        #[arg(long)]
        policy_module: Vec<PathBuf>,
        #[arg(long, default_value_t = 10000)]
        block_time: u64,
        #[arg(long, default_value_t = 1000)]
        reward: u64,
        #[arg(long, default_value_t = 0)]
        premine: u64,
        #[arg(long)]
        premine_to: Option<String>,
        #[arg(long)]
        genesis_transactions: Option<PathBuf>,
    },
    Commit {
        path: String,
        #[arg(long)]
        key: PathBuf,
        #[arg(long)]
        fee: Option<u64>,
        #[arg(long)]
        parent_rpc: Option<String>,
        #[arg(long)]
        parent_cookie: Option<PathBuf>,
        #[arg(long, default_value_t = 3000)]
        poll_ms: u64,
        #[arg(long)]
        timeout_seconds: Option<u64>,
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
    if let Command::Status { json } = &cli.command
        && cli.chain.is_none()
    {
        #[cfg(unix)]
        {
            let report = lattice_ctl::status::read(&cli.root, cli.rpc.as_deref()).await?;
            if *json {
                return output(&report);
            }
            print!("{}", lattice_ctl::status::table(&report));
            return Ok(());
        }
        #[cfg(not(unix))]
        {
            let _ = json;
            anyhow::bail!(
                "tree-wide owned-process status requires Unix; use --chain Nexus status for an explicit RPC query"
            );
        }
    }
    let chain = cli.chain.as_deref().unwrap_or("Nexus");
    match &cli.command {
        #[cfg(unix)]
        Command::Wipe => return output(&lattice_ctl::lifecycle::wipe(&cli.root)?),
        #[cfg(unix)]
        Command::EmitSystemd {
            binary,
            node_binary,
            listen_host,
        } => {
            print!(
                "{}",
                lattice_ctl::service::emit(&cli.root, binary, node_binary, *listen_host)?
            );
            return Ok(());
        }
        #[cfg(unix)]
        Command::Child { command } => {
            return match command {
                ChildCommand::Host { path } => {
                    output(&lattice_ctl::child::host(&cli.root, path).await?)
                }
                ChildCommand::Apply => output(&lattice_ctl::child::apply(&cli.root).await?),
                ChildCommand::Create {
                    path,
                    spec,
                    policy_module,
                    block_time,
                    reward,
                    premine,
                    premine_to,
                    genesis_transactions,
                } => output(
                    &lattice_ctl::child::create(
                        &cli.root,
                        lattice_ctl::child::Create {
                            path: path.clone(),
                            spec: spec.clone(),
                            policy_modules: policy_module.clone(),
                            block_time: *block_time,
                            reward: *reward,
                            premine: *premine,
                            premine_to: premine_to.clone(),
                            genesis_transactions: genesis_transactions.clone(),
                        },
                    )
                    .await?,
                ),
                ChildCommand::Commit {
                    path,
                    key,
                    fee,
                    parent_rpc,
                    parent_cookie,
                    poll_ms,
                    timeout_seconds,
                } => {
                    lattice_ctl::child::commit(
                        &cli.root,
                        lattice_ctl::child::Commit {
                            path: path.clone(),
                            key: key.clone(),
                            fee: *fee,
                            parent_rpc: parent_rpc.clone(),
                            parent_cookie: parent_cookie.clone(),
                            poll_ms: *poll_ms,
                            timeout_seconds: *timeout_seconds,
                        },
                    )
                    .await
                }
            };
        }
        #[cfg(unix)]
        Command::Mine { command } => {
            return match command {
                MineCommand::Start => output(&lattice_ctl::mining::start(&cli.root).await?),
                MineCommand::Run { once, managed } => {
                    lattice_ctl::mining::run(&cli.root, *once, *managed).await
                }
                MineCommand::Stop => output(&lattice_ctl::mining::stop(&cli.root).await?),
                MineCommand::Status => output(&lattice_ctl::mining::status(&cli.root)?),
            };
        }
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
            #[cfg(unix)]
            foreground,
        } => {
            #[cfg(unix)]
            if *foreground {
                return lattice_ctl::lifecycle::supervise(&cli.root, node_binary, *listen_host)
                    .await;
            }
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
        chain,
    )?;
    match cli.command {
        #[cfg(unix)]
        Command::Wipe | Command::EmitSystemd { .. } => unreachable!(),
        #[cfg(unix)]
        Command::Child { .. } => unreachable!(),
        #[cfg(unix)]
        Command::Mine { .. } => unreachable!(),
        Command::Key { .. }
        | Command::Init { .. }
        | Command::Identity
        | Command::Up { .. }
        | Command::Down
        | Command::Topology => unreachable!(),
        Command::Status { .. } => {
            output(&rpc.request(reqwest::Method::GET, "/status", None).await?)
        }
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
            let body = transaction::compose(&signer, command.into(), chain, nonce, fee)?;
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
