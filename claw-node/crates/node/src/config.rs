//! Node configuration and key management.
//!
//! Supports loading from `config.toml` in the data directory.
//! CLI arguments take precedence over config file values.

use anyhow::{Context, Result};
use claw_crypto::keys::generate_keypair;
use serde::Deserialize;
use std::fs;
use std::path::Path;

/// Runtime node config loaded from key.json.
pub struct NodeConfig {
    pub address: [u8; 32],
    pub signing_key_bytes: [u8; 32],
}

/// Persistent config stored in `config.toml`.
#[derive(Debug, Deserialize, Default)]
pub struct FileConfig {
    #[serde(default)]
    pub node: NodeSection,
    #[serde(default)]
    pub network: NetworkSection,
    #[serde(default)]
    pub log: LogSection,
}

#[derive(Debug, Deserialize, Default)]
pub struct NodeSection {
    /// Data directory override (reserved for future use, ignored when loading)
    #[allow(dead_code)]
    pub data_dir: Option<String>,
    /// Network preset: "devnet", "testnet", or "mainnet"
    pub network: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
pub struct NetworkSection {
    /// RPC listen port
    pub rpc_port: Option<u16>,
    /// P2P listen port
    pub p2p_port: Option<u16>,
    /// Additional bootstrap peers (multiaddr format)
    #[serde(default)]
    pub bootstrap: Vec<String>,
    /// Force single-node mode
    pub single: Option<bool>,
}

#[derive(Debug, Deserialize, Default)]
pub struct LogSection {
    /// Log format: "text" or "json"
    pub format: Option<String>,
    /// Log filter directive (e.g. "claw=debug,info")
    pub filter: Option<String>,
}

/// Load config.toml from data directory, returning defaults if file doesn't exist.
pub fn load_file_config(data_dir: &Path) -> Result<FileConfig> {
    let config_path = data_dir.join("config.toml");
    if !config_path.exists() {
        return Ok(FileConfig::default());
    }

    let content = fs::read_to_string(&config_path)
        .with_context(|| format!("read {}", config_path.display()))?;
    let cfg: FileConfig = toml::from_str(&content)
        .with_context(|| format!("parse {}", config_path.display()))?;
    Ok(cfg)
}

/// Write a default config.toml to the data directory (only if it doesn't exist).
pub fn write_default_config(data_dir: &Path, chain_id: &str) -> Result<()> {
    let config_path = data_dir.join("config.toml");
    if config_path.exists() {
        return Ok(());
    }

    let network_name = match chain_id {
        "claw-mainnet-1" => "mainnet",
        "claw-testnet-1" => "testnet",
        _ => "devnet",
    };

    let content = format!(
        r#"# ClawNetwork node configuration
# CLI arguments override these values.

[node]
network = "{network_name}"

[network]
rpc_port = 9710
p2p_port = 9711
# bootstrap = ["/ip4/1.2.3.4/tcp/9711"]
# single = false

[log]
format = "text"
# filter = "claw=info"
"#
    );

    fs::write(&config_path, content).context("write config.toml")?;
    tracing::info!("Wrote default config to {}", config_path.display());
    Ok(())
}

/// Initialize a new node: create data dir, generate keypair, write config.
pub fn init_node(data_dir: &Path, chain_id: &str) -> Result<()> {
    fs::create_dir_all(data_dir).context("create data dir")?;

    let key_path = data_dir.join("key.json");
    if key_path.exists() {
        tracing::info!("Key already exists at {}", key_path.display());
        let cfg = load_config(data_dir)?;
        println!("Address: {}", hex::encode(cfg.address));
        // Still write default config if missing
        write_default_config(data_dir, chain_id)?;
        return Ok(());
    }

    let (sk, vk) = generate_keypair();
    let addr = vk.to_bytes();

    let key_data = serde_json::json!({
        "address": hex::encode(addr),
        "secret_key": hex::encode(sk.to_bytes()),
        "chain_id": chain_id,
    });

    fs::write(&key_path, serde_json::to_string_pretty(&key_data)?)
        .context("write key file")?;

    // Restrict permissions (Unix only)
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&key_path, fs::Permissions::from_mode(0o600))?;
    }

    // Write default config.toml
    write_default_config(data_dir, chain_id)?;

    println!("Node initialized at {}", data_dir.display());
    println!("Address: {}", hex::encode(addr));

    Ok(())
}

/// Load node config from data dir.
pub fn load_config(data_dir: &Path) -> Result<NodeConfig> {
    let key_path = data_dir.join("key.json");
    let content = fs::read_to_string(&key_path).context("read key file")?;
    let json: serde_json::Value = serde_json::from_str(&content)?;

    let addr_hex = json["address"].as_str().context("missing address")?;
    let sk_hex = json["secret_key"].as_str().context("missing secret_key")?;

    let addr_bytes = hex::decode(addr_hex)?;
    let sk_bytes = hex::decode(sk_hex)?;

    let mut address = [0u8; 32];
    address.copy_from_slice(&addr_bytes);

    let mut signing_key_bytes = [0u8; 32];
    signing_key_bytes.copy_from_slice(&sk_bytes);

    Ok(NodeConfig {
        address,
        signing_key_bytes,
    })
}
