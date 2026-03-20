//! ClawNetwork node binary.

mod chain;
mod config;
mod genesis;
pub(crate) mod metrics;
mod network;
mod rpc_server;
mod sync;

use anyhow::Result;
use clap::{Parser, Subcommand, ValueEnum};
use libp2p::Multiaddr;
use std::path::PathBuf;
use tracing_subscriber::{fmt, EnvFilter, prelude::*};

use network::Network;

#[derive(Parser)]
#[command(name = "claw-node", version, about = "ClawNetwork AI Agent Blockchain Node")]
struct Cli {
    #[command(subcommand)]
    command: Commands,

    /// Data directory
    #[arg(long, default_value = "~/.clawnetwork", global = true)]
    data_dir: String,

    /// Log output format
    #[arg(long, global = true)]
    log_format: Option<LogFormat>,
}

#[derive(Clone, ValueEnum)]
enum LogFormat {
    /// Human-readable text output
    Text,
    /// Structured JSON output
    Json,
}

#[derive(Subcommand)]
enum Commands {
    /// Initialize node: generate keypair and config
    Init {
        /// Network to join
        #[arg(long, default_value = "devnet")]
        network: Network,
    },
    /// Start the node
    Start {
        /// Network to join (devnet = local single-node, testnet/mainnet = P2P)
        #[arg(long, short = 'n')]
        network: Option<Network>,
        /// RPC port
        #[arg(long)]
        rpc_port: Option<u16>,
        /// P2P port
        #[arg(long)]
        p2p_port: Option<u16>,
        /// Additional bootstrap peer addresses (multiaddr format)
        #[arg(long)]
        bootstrap: Vec<String>,
        /// Force single-node mode (no P2P, overrides network preset)
        #[arg(long)]
        single: bool,
        /// Sync mode: full (all blocks), fast (state snapshot + recent), light (prune old blocks)
        #[arg(long, default_value = "full")]
        sync_mode: String,
    },
    /// Show node status
    Status,
    /// Key management
    Key {
        #[command(subcommand)]
        action: KeyAction,
    },
    /// Encrypt an existing plaintext key.json (requires CLAW_KEY_PASSWORD env var)
    EncryptKey,
    /// Export the default genesis config for a network as JSON
    Genesis {
        /// Network preset to export
        #[arg(long, default_value = "devnet")]
        network: Network,
    },
    /// Transfer CLAW to another address
    Transfer {
        /// Recipient address (hex, 64 chars)
        to: String,
        /// Amount in CLW (e.g. "10000" or "0.5")
        amount: String,
        /// RPC endpoint URL
        #[arg(long, default_value = "http://localhost:9710")]
        rpc: String,
    },
    /// Stake CLAW to become a validator
    Stake {
        /// Amount in CLW to stake (e.g. "10000")
        amount: String,
        /// RPC endpoint URL
        #[arg(long, default_value = "http://localhost:9710")]
        rpc: String,
    },
    /// Smart contract queries (reads from a running node via RPC)
    Contract {
        #[command(subcommand)]
        action: ContractAction,
        /// RPC endpoint URL
        #[arg(long, default_value = "http://localhost:9710")]
        rpc: String,
    },
}

#[derive(Subcommand)]
enum KeyAction {
    /// Generate a new keypair
    Generate,
    /// Show current address
    Show,
}

#[derive(Subcommand)]
enum ContractAction {
    /// Get contract metadata by address
    Info {
        /// Contract address (hex, 64 chars)
        address: String,
    },
    /// Get a storage value from a contract
    Storage {
        /// Contract address (hex, 64 chars)
        address: String,
        /// Storage key (hex)
        key: String,
    },
    /// Get contract Wasm bytecode
    Code {
        /// Contract address (hex, 64 chars)
        address: String,
    },
    /// Execute a read-only contract view call
    Call {
        /// Contract address (hex, 64 chars)
        address: String,
        /// Method name to call
        method: String,
        /// Arguments as hex-encoded bytes (optional)
        #[arg(default_value = "")]
        args: String,
    },
}

fn expand_path(path: &str) -> PathBuf {
    if path.starts_with("~/") {
        if let Some(home) = dirs_next::home_dir() {
            return home.join(&path[2..]);
        }
    }
    PathBuf::from(path)
}

fn init_logging(log_format: &LogFormat, filter_override: Option<&str>) -> Result<()> {
    let filter = match filter_override {
        Some(f) => EnvFilter::from_default_env().add_directive(f.parse()?),
        None => EnvFilter::from_default_env().add_directive("claw=info".parse()?),
    };

    match log_format {
        LogFormat::Text => {
            tracing_subscriber::registry()
                .with(filter)
                .with(fmt::layer())
                .init();
        }
        LogFormat::Json => {
            tracing_subscriber::registry()
                .with(filter)
                .with(fmt::layer().json())
                .init();
        }
    }
    Ok(())
}

/// Parse a Network variant from a string (for config.toml values).
fn parse_network(s: &str) -> Option<Network> {
    match s.to_lowercase().as_str() {
        "mainnet" => Some(Network::Mainnet),
        "testnet" => Some(Network::Testnet),
        "devnet" => Some(Network::Devnet),
        _ => None,
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let data_dir = expand_path(&cli.data_dir);

    // Load config.toml (may not exist yet — returns defaults)
    let file_cfg = config::load_file_config(&data_dir).unwrap_or_default();

    // Resolve log format: CLI > config.toml > default "text"
    let log_format = cli.log_format.unwrap_or_else(|| {
        match file_cfg.log.format.as_deref() {
            Some("json") => LogFormat::Json,
            _ => LogFormat::Text,
        }
    });

    init_logging(&log_format, file_cfg.log.filter.as_deref())?;

    match cli.command {
        Commands::Init { network } => {
            let net_cfg = network.config();
            config::init_node(&data_dir, net_cfg.chain_id)?;
        }
        Commands::Start {
            network,
            rpc_port,
            p2p_port,
            bootstrap,
            single,
            sync_mode,
        } => {
            let sync_mode = sync::SyncMode::parse(&sync_mode);
            // Resolve network: CLI > config.toml > default devnet
            let resolved_network = network.unwrap_or_else(|| {
                file_cfg
                    .node
                    .network
                    .as_deref()
                    .and_then(parse_network)
                    .unwrap_or(Network::Devnet)
            });
            let net_cfg = resolved_network.config();

            // Resolve ports: CLI > config.toml > defaults
            let resolved_rpc_port = rpc_port
                .or(file_cfg.network.rpc_port)
                .unwrap_or(9710);
            let resolved_p2p_port = p2p_port
                .or(file_cfg.network.p2p_port)
                .unwrap_or(9711);
            let resolved_single = single || file_cfg.network.single.unwrap_or(false);

            // Ensure initialized
            if !data_dir.join("key.json").exists() {
                config::init_node(&data_dir, net_cfg.chain_id)?;
            }

            let cfg = config::load_config(&data_dir)?;
            tracing::info!(
                address = %hex::encode(cfg.address),
                network = net_cfg.chain_id,
                rpc_port = resolved_rpc_port,
                p2p_port = resolved_p2p_port,
                sync_mode = %sync_mode,
                "Starting claw-node"
            );

            // Load genesis config: genesis.json in data_dir > built-in default
            let network_name = match resolved_network {
                Network::Mainnet => "mainnet",
                Network::Testnet => "testnet",
                Network::Devnet => "devnet",
            };
            let genesis_cfg = genesis::load_genesis_config(
                &data_dir,
                network_name,
                Some(&cfg.address),
            )?;

            let chain = chain::Chain::new(&data_dir, cfg.signing_key_bytes, &genesis_cfg)?;

            // Fast sync: log intent (actual snapshot request happens on first peer connection)
            if sync_mode == sync::SyncMode::Fast {
                sync::log_fast_sync_intent();
            }

            // Light mode: spawn periodic pruning task
            let _prune_handle = if sync_mode == sync::SyncMode::Light {
                let prune_chain = chain.clone();
                let prune_dir = data_dir.clone();
                Some(tokio::spawn(async move {
                    sync::run_light_pruning_loop(prune_chain, &prune_dir).await;
                }))
            } else {
                None
            };

            // Start RPC server
            let rpc_handle = rpc_server::start(chain.clone(), resolved_rpc_port, net_cfg.faucet_enabled).await?;
            tracing::info!(port = resolved_rpc_port, "RPC server listening");

            // Determine if we should run in single-node mode
            let run_single = resolved_single || net_cfg.is_local;

            if run_single {
                tracing::info!("Running in single-node mode (no P2P)");
                chain.run_block_loop().await;
            } else {
                // Merge preset bootstrap + config.toml bootstrap + CLI bootstrap
                let mut all_bootstrap: Vec<String> = net_cfg
                    .bootstrap_peers
                    .iter()
                    .map(|s| s.to_string())
                    .collect();
                all_bootstrap.extend(file_cfg.network.bootstrap);
                all_bootstrap.extend(bootstrap);

                let bootstrap_addrs: Vec<Multiaddr> = all_bootstrap
                    .iter()
                    .filter_map(|s| s.parse().ok())
                    .collect();

                if bootstrap_addrs.is_empty() {
                    tracing::warn!("No bootstrap peers configured — running as solo node. Use --bootstrap to connect.");
                }

                match claw_p2p::P2pNetwork::new(resolved_p2p_port, bootstrap_addrs) {
                    Ok((mut p2p, event_rx, command_tx)) => {
                        tracing::info!(port = resolved_p2p_port, peers = all_bootstrap.len(), "P2P network started");

                        let p2p_handle = tokio::spawn(async move {
                            p2p.run().await;
                        });

                        tracing::info!("Running with P2P networking");

                        let chain_clone = chain.clone();
                        let event_handle = tokio::spawn(async move {
                            chain_clone.run_p2p_events(event_rx, command_tx).await;
                        });

                        chain.run_block_loop().await;

                        p2p_handle.abort();
                        event_handle.abort();
                    }
                    Err(e) => {
                        tracing::warn!(error = %e, "Failed to start P2P, falling back to single-node");
                        chain.run_block_loop().await;
                    }
                }
            }

            rpc_handle.abort();
        }
        Commands::Status => {
            println!("claw-node status: use RPC at http://localhost:9710");
        }
        Commands::Key { action } => match action {
            KeyAction::Generate => {
                config::init_node(&data_dir, "claw-devnet")?;
            }
            KeyAction::Show => {
                let cfg = config::load_config(&data_dir)?;
                println!("Address: {}", hex::encode(cfg.address));
            }
        },
        Commands::EncryptKey => {
            config::encrypt_existing_key(&data_dir)?;
        }
        Commands::Genesis { network } => {
            let network_name = match network {
                Network::Mainnet => "mainnet",
                Network::Testnet => "testnet",
                Network::Devnet => "devnet",
            };
            // Try to load node address for devnet defaults
            let node_address = config::load_config(&data_dir)
                .ok()
                .map(|c| c.address);
            let config = genesis::default_for_network(
                network_name,
                node_address.as_ref(),
            );
            let json = genesis::export_json(&config)?;
            println!("{json}");
        }
        Commands::Transfer { to, amount, rpc } => {
            handle_transfer_cli(&data_dir, &to, &amount, &rpc).await?;
        }
        Commands::Stake { amount, rpc } => {
            handle_stake_cli(&data_dir, &amount, &rpc).await?;
        }
        Commands::Contract { action, rpc } => {
            handle_contract_cli(action, &rpc).await?;
        }
    }

    Ok(())
}

/// Parse a CLW amount string (supports decimals, e.g. "10000" or "0.5") into raw u128 (9 decimals).
fn parse_clw_amount(s: &str) -> Result<u128> {
    let s = s.trim();
    let (whole, frac) = if let Some(dot) = s.find('.') {
        let whole: u128 = s[..dot].parse().map_err(|e| anyhow::anyhow!("invalid amount: {e}"))?;
        let frac_str = &s[dot + 1..];
        let frac_len = frac_str.len();
        if frac_len > 9 {
            anyhow::bail!("too many decimal places (max 9)");
        }
        let frac: u128 = frac_str.parse().map_err(|e| anyhow::anyhow!("invalid decimal: {e}"))?;
        let frac_scaled = frac * 10u128.pow(9 - frac_len as u32);
        (whole, frac_scaled)
    } else {
        let whole: u128 = s.parse().map_err(|e| anyhow::anyhow!("invalid amount: {e}"))?;
        (whole, 0)
    };
    Ok(whole * 1_000_000_000 + frac)
}

/// Parse a hex address string into [u8; 32].
fn parse_hex_address(s: &str) -> Result<[u8; 32]> {
    let bytes = hex::decode(s).map_err(|e| anyhow::anyhow!("invalid hex address: {e}"))?;
    if bytes.len() != 32 {
        anyhow::bail!("address must be 64 hex chars (32 bytes), got {}", bytes.len());
    }
    let mut addr = [0u8; 32];
    addr.copy_from_slice(&bytes);
    Ok(addr)
}

/// Build, sign, and submit a transaction via RPC.
async fn submit_tx(
    data_dir: &std::path::Path,
    rpc: &str,
    tx_type: claw_types::transaction::TxType,
    payload_bytes: Vec<u8>,
) -> Result<String> {
    use claw_types::transaction::Transaction;

    let cfg = config::load_config(data_dir)?;
    let from = cfg.address;

    // Get current nonce
    let nonce_result = rpc_call(rpc, "clw_getNonce", vec![serde_json::json!(hex::encode(from))]).await?;
    let current_nonce: u64 = nonce_result.as_u64().unwrap_or(0);
    let nonce = current_nonce + 1;

    // Build transaction
    let mut tx = Transaction {
        tx_type,
        from,
        nonce,
        payload: payload_bytes,
        signature: [0u8; 64],
    };

    // Sign
    let signing_key = claw_crypto::ed25519_dalek::SigningKey::from_bytes(&cfg.signing_key_bytes);
    claw_crypto::signer::sign_transaction(&mut tx, &signing_key);

    // Serialize
    let tx_hex = hex::encode(borsh::to_vec(&tx)?);

    // Submit
    let result = rpc_call(rpc, "clw_sendTransaction", vec![serde_json::json!(tx_hex)]).await?;
    let tx_hash = result.as_str().unwrap_or("unknown").to_string();

    Ok(tx_hash)
}

async fn handle_transfer_cli(data_dir: &std::path::Path, to: &str, amount: &str, rpc: &str) -> Result<()> {
    use claw_types::transaction::{TokenTransferPayload, TxType};

    let to_addr = parse_hex_address(to)?;
    let raw_amount = parse_clw_amount(amount)?;

    let cfg = config::load_config(data_dir)?;
    let from_hex = hex::encode(cfg.address);

    // Check balance
    let balance_result = rpc_call(rpc, "clw_getBalance", vec![serde_json::json!(&from_hex)]).await?;
    let balance: u128 = balance_result.as_str()
        .and_then(|s| s.parse().ok())
        .or_else(|| balance_result.as_u64().map(|v| v as u128))
        .unwrap_or(0);

    if balance < raw_amount {
        let balance_clw = balance as f64 / 1_000_000_000.0;
        anyhow::bail!("insufficient balance: have {:.4} CLW, need {} CLW", balance_clw, amount);
    }

    println!("Transfer {} CLW", amount);
    println!("  From: {}", from_hex);
    println!("  To:   {}", to);
    println!("  Raw:  {} (9 decimals)", raw_amount);

    let payload = TokenTransferPayload {
        to: to_addr,
        amount: raw_amount,
    };
    let payload_bytes = borsh::to_vec(&payload)?;

    let tx_hash = submit_tx(data_dir, rpc, TxType::TokenTransfer, payload_bytes).await?;
    println!("  TX:   {}", tx_hash);
    println!("  Status: submitted (confirms in ~3s)");

    Ok(())
}

async fn handle_stake_cli(data_dir: &std::path::Path, amount: &str, rpc: &str) -> Result<()> {
    use claw_types::transaction::{StakeDepositPayload, TxType};

    let raw_amount = parse_clw_amount(amount)?;

    let cfg = config::load_config(data_dir)?;
    let from_hex = hex::encode(cfg.address);

    // Check balance
    let balance_result = rpc_call(rpc, "clw_getBalance", vec![serde_json::json!(&from_hex)]).await?;
    let balance: u128 = balance_result.as_str()
        .and_then(|s| s.parse().ok())
        .or_else(|| balance_result.as_u64().map(|v| v as u128))
        .unwrap_or(0);

    if balance < raw_amount {
        let balance_clw = balance as f64 / 1_000_000_000.0;
        anyhow::bail!("insufficient balance: have {:.4} CLW, need {} CLW", balance_clw, amount);
    }

    println!("Stake {} CLW", amount);
    println!("  Validator: {}", from_hex);
    println!("  Raw:       {} (9 decimals)", raw_amount);

    let payload = StakeDepositPayload {
        amount: raw_amount,
    };
    let payload_bytes = borsh::to_vec(&payload)?;

    let tx_hash = submit_tx(data_dir, rpc, TxType::StakeDeposit, payload_bytes).await?;
    println!("  TX:   {}", tx_hash);
    println!("  Status: submitted (active after next epoch)");

    Ok(())
}

/// Send a JSON-RPC request and return the result value.
async fn rpc_call(url: &str, method: &str, params: Vec<serde_json::Value>) -> Result<serde_json::Value> {
    let client = reqwest::Client::new();
    let body = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": method,
        "params": params,
    });
    let resp = client
        .post(url)
        .json(&body)
        .send()
        .await?
        .json::<serde_json::Value>()
        .await?;
    if let Some(err) = resp.get("error") {
        anyhow::bail!("RPC error: {}", err);
    }
    Ok(resp.get("result").cloned().unwrap_or(serde_json::Value::Null))
}

async fn handle_contract_cli(action: ContractAction, rpc_url: &str) -> Result<()> {
    match action {
        ContractAction::Info { address } => {
            let result = rpc_call(rpc_url, "clw_getContractInfo", vec![address.into()]).await?;
            if result.is_null() {
                println!("Contract not found");
            } else {
                println!("{}", serde_json::to_string_pretty(&result)?);
            }
        }
        ContractAction::Storage { address, key } => {
            let result = rpc_call(rpc_url, "clw_getContractStorage", vec![address.into(), key.into()]).await?;
            if result.is_null() {
                println!("Storage key not found");
            } else {
                println!("{}", result.as_str().unwrap_or(&result.to_string()));
            }
        }
        ContractAction::Code { address } => {
            let result = rpc_call(rpc_url, "clw_getContractCode", vec![address.into()]).await?;
            if result.is_null() {
                println!("Contract not found");
            } else {
                let size = result.get("size").and_then(|v| v.as_u64()).unwrap_or(0);
                println!("Code size: {} bytes", size);
                if let Some(code) = result.get("code").and_then(|v| v.as_str()) {
                    // Truncate display for large bytecode
                    if code.len() > 128 {
                        println!("Code (first 64 bytes): {}...", &code[..128]);
                    } else {
                        println!("Code: {}", code);
                    }
                }
            }
        }
        ContractAction::Call { address, method, args } => {
            let args_param = if args.is_empty() { "" } else { &args };
            let result = rpc_call(
                rpc_url,
                "clw_callContractView",
                vec![address.into(), method.into(), args_param.into()],
            )
            .await?;
            println!("{}", serde_json::to_string_pretty(&result)?);
        }
    }
    Ok(())
}
