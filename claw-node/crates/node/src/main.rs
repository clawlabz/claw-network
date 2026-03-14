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
}

#[derive(Subcommand)]
enum KeyAction {
    /// Generate a new keypair
    Generate,
    /// Show current address
    Show,
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

            let chain = chain::Chain::new(&data_dir, cfg.signing_key_bytes)?;

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
    }

    Ok(())
}
