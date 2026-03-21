//! Genesis block generation with configurable genesis.json support.

use anyhow::{bail, Context, Result};
use claw_consensus::MIN_STAKE;
use claw_state::WorldState;
use claw_types::block::Block;
use claw_types::state::CLW_TOTAL_SUPPLY;
use serde::{Deserialize, Serialize};
use std::path::Path;

/// Genesis configuration loaded from genesis.json or built-in defaults.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GenesisConfig {
    pub chain_id: String,
    pub timestamp: u64,
    pub allocations: Vec<GenesisAllocation>,
    pub validators: Vec<GenesisValidator>,
}

/// A genesis token allocation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GenesisAllocation {
    /// Hex-encoded [u8; 32] address.
    pub address: String,
    /// Decimal string in base units (9 decimals).
    pub balance: String,
    /// Human-readable label (e.g. "node_incentives", "ecosystem_fund").
    pub label: String,
}

/// A genesis validator entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GenesisValidator {
    /// Hex-encoded [u8; 32] address.
    pub address: String,
    /// Decimal string in base units (9 decimals).
    pub stake: String,
}

// ---------------------------------------------------------------------------
// Address helpers
// ---------------------------------------------------------------------------

/// Deterministic genesis allocation address from index.
fn genesis_address(index: u8) -> [u8; 32] {
    let mut addr = [0u8; 32];
    addr[0] = index;
    addr
}

/// Encode a genesis address index as a hex string.
fn genesis_address_hex(index: u8) -> String {
    hex::encode(genesis_address(index))
}

// ---------------------------------------------------------------------------
// Default configs per network
// ---------------------------------------------------------------------------

/// Build the standard 5-way allocation (40/25/15/10/10) for mainnet/testnet.
fn standard_allocations() -> Vec<GenesisAllocation> {
    vec![
        GenesisAllocation {
            address: genesis_address_hex(1),
            balance: (CLW_TOTAL_SUPPLY * 40 / 100).to_string(),
            label: "node_incentives".into(),
        },
        GenesisAllocation {
            address: genesis_address_hex(2),
            balance: (CLW_TOTAL_SUPPLY * 25 / 100).to_string(),
            label: "ecosystem_fund".into(),
        },
        GenesisAllocation {
            address: "71fa1a514e07c7c96bf0c825c29dfc8059cfa995318972dd258a4e316873e66b".into(),
            balance: (CLW_TOTAL_SUPPLY * 15 / 100).to_string(),
            label: "team".into(),
        },
        GenesisAllocation {
            address: genesis_address_hex(4),
            balance: (CLW_TOTAL_SUPPLY * 10 / 100).to_string(),
            label: "early_contributors".into(),
        },
        GenesisAllocation {
            address: genesis_address_hex(5),
            balance: (CLW_TOTAL_SUPPLY * 10 / 100).to_string(),
            label: "liquidity_reserve".into(),
        },
    ]
}

/// Default genesis config for devnet (single node, local development).
///
/// Uses the node's own address as the sole validator with 1M CLAW stake
/// and a faucet allocation. Also includes the standard 5-way distribution.
pub fn default_devnet(node_address: Option<&[u8; 32]>) -> GenesisConfig {
    let node_addr_hex = node_address
        .map(hex::encode)
        .unwrap_or_else(|| genesis_address_hex(10));

    let faucet_amount: u128 = 1_000_000_000_000_000; // 1M CLAW
    let validator_stake: u128 = 1_000_000_000_000_000; // 1M CLAW

    let mut allocations = standard_allocations();
    allocations.push(GenesisAllocation {
        address: node_addr_hex.clone(),
        balance: faucet_amount.to_string(),
        label: "devnet_faucet".into(),
    });

    GenesisConfig {
        chain_id: "claw-devnet".into(),
        timestamp: 1741737600, // 2025-03-12 00:00:00 UTC
        allocations,
        validators: vec![GenesisValidator {
            address: node_addr_hex,
            stake: validator_stake.to_string(),
        }],
    }
}

/// Default genesis config for testnet.
pub fn default_testnet() -> GenesisConfig {
    GenesisConfig {
        chain_id: "claw-testnet-1".into(),
        timestamp: 1741737600,
        allocations: standard_allocations(),
        validators: vec![
            // Bootstrap validator operated by ClawLabz
            GenesisValidator {
                address: genesis_address_hex(20),
                stake: (MIN_STAKE * 100).to_string(),
            },
        ],
    }
}

/// Default genesis config for mainnet.
pub fn default_mainnet() -> GenesisConfig {
    GenesisConfig {
        chain_id: "claw-mainnet-1".into(),
        timestamp: 1742515200, // 2025-03-21 00:00:00 UTC
        allocations: standard_allocations(),
        validators: vec![
            // Hetzner genesis validator operated by ClawLabz
            GenesisValidator {
                address: "ffa28f7c6469ab7490ce540a0e49aa64bc77b4dc5bb2a83b17ddd10a9c8ea62e".into(),
                stake: (MIN_STAKE * 100).to_string(), // 1,000,000 CLAW
            },
        ],
    }
}

/// Return the built-in default genesis config for a network name.
pub fn default_for_network(network: &str, node_address: Option<&[u8; 32]>) -> GenesisConfig {
    match network {
        "mainnet" | "claw-mainnet-1" => default_mainnet(),
        "testnet" | "claw-testnet-1" => default_testnet(),
        _ => default_devnet(node_address),
    }
}

// ---------------------------------------------------------------------------
// Validation
// ---------------------------------------------------------------------------

/// Parse a hex-encoded address string into [u8; 32].
fn parse_address(hex_str: &str, context: &str) -> Result<[u8; 32]> {
    let bytes = hex::decode(hex_str)
        .with_context(|| format!("invalid hex in {context}: {hex_str}"))?;
    if bytes.len() != 32 {
        bail!(
            "{context}: address has length {} (expected 32 bytes / 64 hex chars)",
            bytes.len()
        );
    }
    let mut addr = [0u8; 32];
    addr.copy_from_slice(&bytes);
    Ok(addr)
}

/// Parse a decimal balance string into u128.
fn parse_balance(s: &str, context: &str) -> Result<u128> {
    s.parse::<u128>()
        .with_context(|| format!("{context}: invalid balance \"{s}\""))
}

/// Validate a GenesisConfig: total supply check, address format, minimum stake.
pub fn validate(config: &GenesisConfig) -> Result<()> {
    // Validate allocations
    let mut total: u128 = 0;
    for (i, alloc) in config.allocations.iter().enumerate() {
        let label = format!("allocation[{}] ({})", i, alloc.label);
        parse_address(&alloc.address, &label)?;
        let balance = parse_balance(&alloc.balance, &label)?;
        total = total
            .checked_add(balance)
            .with_context(|| format!("{label}: total supply overflow"))?;
    }

    // Validate validators
    for (i, val) in config.validators.iter().enumerate() {
        let label = format!("validator[{}]", i);
        parse_address(&val.address, &label)?;
        let stake = parse_balance(&val.stake, &label)?;
        if stake < MIN_STAKE {
            bail!(
                "{label}: stake {} is below MIN_STAKE {}",
                stake,
                MIN_STAKE
            );
        }
    }

    // Total supply check
    if total != CLW_TOTAL_SUPPLY {
        bail!(
            "total allocation {} does not equal CLW_TOTAL_SUPPLY {}",
            total,
            CLW_TOTAL_SUPPLY
        );
    }

    if config.validators.is_empty() {
        bail!("genesis config must have at least one validator");
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Loading
// ---------------------------------------------------------------------------

/// Load genesis config from the data directory.
///
/// Priority:
/// 1. `genesis.json` in `data_dir` — parsed and validated
/// 2. Built-in default for the given network
pub fn load_genesis_config(
    data_dir: &Path,
    network: &str,
    node_address: Option<&[u8; 32]>,
) -> Result<GenesisConfig> {
    let genesis_path = data_dir.join("genesis.json");
    if genesis_path.exists() {
        tracing::info!(path = %genesis_path.display(), "Loading genesis config from file");
        let content = std::fs::read_to_string(&genesis_path)
            .with_context(|| format!("read {}", genesis_path.display()))?;
        let config: GenesisConfig = serde_json::from_str(&content)
            .with_context(|| format!("parse {}", genesis_path.display()))?;
        validate(&config)?;
        return Ok(config);
    }

    tracing::info!(
        network,
        "No genesis.json found, using built-in default"
    );
    let config = default_for_network(network, node_address);
    // Built-in devnet config includes faucet allocation beyond CLW_TOTAL_SUPPLY,
    // so we skip strict validation for devnet defaults.
    if network != "devnet" && network != "claw-devnet" {
        validate(&config)?;
    }
    Ok(config)
}

// ---------------------------------------------------------------------------
// State / block creation
// ---------------------------------------------------------------------------

/// Create the genesis world state from a GenesisConfig.
pub fn create_genesis_state(config: &GenesisConfig) -> Result<WorldState> {
    let mut state = WorldState::default();
    state.block_height = 0;

    for alloc in &config.allocations {
        let addr = parse_address(&alloc.address, &alloc.label)?;
        let balance = parse_balance(&alloc.balance, &alloc.label)?;
        *state.balances.entry(addr).or_insert(0) += balance;
    }

    Ok(state)
}

/// Create the genesis block with chain_id stored in the block's metadata.
pub fn create_genesis_block(state: &WorldState, config: &GenesisConfig) -> Block {
    let state_root = state.state_root();
    let mut block = Block {
        height: 0,
        prev_hash: [0u8; 32],
        timestamp: config.timestamp,
        validator: [0u8; 32],
        transactions: vec![],
        state_root,
        hash: [0u8; 32],
        signatures: Vec::new(),
        events: Vec::new(),
    };
    block.hash = block.compute_hash();
    block
}

/// Build the initial validator set from genesis config.
pub fn build_validator_set(
    config: &GenesisConfig,
) -> Result<Vec<([u8; 32], u128)>> {
    let mut stakes = Vec::with_capacity(config.validators.len());
    for val in &config.validators {
        let addr = parse_address(&val.address, "validator")?;
        let stake = parse_balance(&val.stake, "validator stake")?;
        stakes.push((addr, stake));
    }
    Ok(stakes)
}

/// Export a genesis config as pretty-printed JSON.
pub fn export_json(config: &GenesisConfig) -> Result<String> {
    serde_json::to_string_pretty(config).context("serialize genesis config")
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_testnet_validates() {
        let config = default_testnet();
        validate(&config).expect("testnet config should be valid");
    }

    #[test]
    fn default_mainnet_validates() {
        let config = default_mainnet();
        validate(&config).expect("mainnet config should be valid");
    }

    #[test]
    fn devnet_skips_supply_check_due_to_faucet() {
        let addr = [42u8; 32];
        let config = default_devnet(Some(&addr));
        // Devnet has faucet allocation beyond CLW_TOTAL_SUPPLY, so strict
        // validation would fail — this is expected.
        assert!(validate(&config).is_err());
    }

    #[test]
    fn create_state_from_config() {
        let config = default_testnet();
        let state = create_genesis_state(&config).unwrap();
        // Check that all 5 allocations exist
        assert_eq!(state.balances.len(), 5);
    }

    #[test]
    fn invalid_address_rejected() {
        let config = GenesisConfig {
            chain_id: "test".into(),
            timestamp: 0,
            allocations: vec![GenesisAllocation {
                address: "not_hex".into(),
                balance: "100".into(),
                label: "bad".into(),
            }],
            validators: vec![GenesisValidator {
                address: genesis_address_hex(1),
                stake: MIN_STAKE.to_string(),
            }],
        };
        assert!(validate(&config).is_err());
    }

    #[test]
    fn below_min_stake_rejected() {
        let config = GenesisConfig {
            chain_id: "test".into(),
            timestamp: 0,
            allocations: standard_allocations(),
            validators: vec![GenesisValidator {
                address: genesis_address_hex(1),
                stake: "1".into(), // way below MIN_STAKE
            }],
        };
        assert!(validate(&config).is_err());
    }

    #[test]
    fn roundtrip_json() {
        let config = default_testnet();
        let json = export_json(&config).unwrap();
        let parsed: GenesisConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.chain_id, config.chain_id);
        assert_eq!(parsed.allocations.len(), config.allocations.len());
    }

    #[test]
    fn build_validator_set_works() {
        let config = default_testnet();
        let stakes = build_validator_set(&config).unwrap();
        assert_eq!(stakes.len(), 1);
        assert!(stakes[0].1 >= MIN_STAKE);
    }
}
