//! World state types: agent identity, tokens, reputation, services.

use borsh::{BorshDeserialize, BorshSerialize};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Registered Agent identity on-chain.
#[derive(Debug, Clone, PartialEq, Eq, BorshSerialize, BorshDeserialize, Serialize, Deserialize)]
pub struct AgentIdentity {
    /// Ed25519 public key / address.
    pub address: [u8; 32],
    /// Human-readable name.
    pub name: String,
    /// Arbitrary key-value metadata (e.g., platform associations).
    pub metadata: BTreeMap<String, String>,
    /// Block height at which the agent was registered.
    pub registered_at: u64,
}

/// A custom token definition.
#[derive(Debug, Clone, PartialEq, Eq, BorshSerialize, BorshDeserialize, Serialize, Deserialize)]
pub struct TokenDef {
    /// Token ID: blake3(creator_address || name || nonce).
    pub id: [u8; 32],
    /// Token name.
    pub name: String,
    /// Token symbol (e.g., "MSC").
    pub symbol: String,
    /// Decimal places.
    pub decimals: u8,
    /// Total supply (minted to issuer on creation).
    pub total_supply: u128,
    /// Creator/issuer agent address.
    pub issuer: [u8; 32],
}

/// An on-chain reputation attestation.
#[derive(Debug, Clone, PartialEq, Eq, BorshSerialize, BorshDeserialize, Serialize, Deserialize)]
pub struct ReputationAttestation {
    /// Address of the attester.
    pub from: [u8; 32],
    /// Address of the agent being attested.
    pub to: [u8; 32],
    /// Category (e.g., "game", "task", "service").
    pub category: String,
    /// Score from -100 to +100.
    pub score: i16,
    /// Source platform (arbitrary string, e.g., "clawarena").
    pub platform: String,
    /// Optional memo.
    pub memo: String,
    /// Block height at which this attestation was recorded.
    pub block_height: u64,
}

/// A service entry in the on-chain service registry.
#[derive(Debug, Clone, PartialEq, Eq, BorshSerialize, BorshDeserialize, Serialize, Deserialize)]
pub struct ServiceEntry {
    /// Provider agent address.
    pub provider: [u8; 32],
    /// Service type (e.g., "translation", "code-review").
    pub service_type: String,
    /// Human-readable description.
    pub description: String,
    /// Token accepted for payment (CLAW native = all zeros).
    pub price_token: [u8; 32],
    /// Price amount per unit of service.
    pub price_amount: u128,
    /// Endpoint URL for the service.
    pub endpoint: String,
    /// Whether the service is currently active.
    pub active: bool,
}

/// An entry in the unbonding queue for a validator withdrawing stake.
#[derive(Debug, Clone, PartialEq, Eq, BorshSerialize, BorshDeserialize, Serialize, Deserialize)]
pub struct UnbondingEntry {
    /// Validator address that initiated the unbonding.
    pub address: [u8; 32],
    /// Amount of CLAW being unbonded.
    pub amount: u128,
    /// Block height at which the unbonded stake can be claimed.
    pub release_height: u64,
}

/// Unbonding period in blocks: 7 days at 3-second block time.
/// 7 * 24 * 3600 / 3 = 201,600 blocks.
pub const UNBONDING_PERIOD_BLOCKS: u64 = 201_600;

/// Per-epoch on-chain activity statistics for an address.
#[derive(Debug, Clone, PartialEq, Eq, Default, BorshSerialize, BorshDeserialize, Serialize, Deserialize)]
pub struct ActivityStats {
    /// Number of transactions sent by this address in the current epoch.
    pub tx_count: u32,
    /// Number of contract deployments.
    pub contract_deploys: u32,
    /// Number of contract calls.
    pub contract_calls: u32,
    /// Number of tokens created.
    pub tokens_created: u32,
    /// Number of services registered.
    pub services_registered: u32,
    /// Total gas consumed (in base units).
    pub gas_consumed: u64,
}

/// Validator uptime tracking within a sliding window.
#[derive(Debug, Clone, PartialEq, Eq, Default, BorshSerialize, BorshDeserialize, Serialize, Deserialize)]
pub struct ValidatorUptime {
    /// Number of blocks this validator signed within the window.
    pub signed_blocks: u64,
    /// Number of blocks this validator was expected to sign within the window.
    pub expected_blocks: u64,
    /// Number of blocks this validator produced within the window.
    pub produced_blocks: u64,
}

/// Aggregated platform activity report data per agent, per epoch.
#[derive(Debug, Clone, PartialEq, Eq, Default, BorshSerialize, BorshDeserialize, Serialize, Deserialize)]
pub struct PlatformActivityAgg {
    /// Total action count across all platform reports.
    pub total_actions: u64,
    /// Number of distinct platforms that reported for this agent.
    pub platform_count: u32,
}

/// Maximum action_type length in bytes for PlatformActivityReport.
pub const MAX_ACTION_TYPE_LEN: usize = 64;

/// Minimum stake required for a Platform Agent to submit activity reports.
/// 50,000 CLAW with 9 decimals.
pub const PLATFORM_AGENT_MIN_STAKE: u128 = 50_000_000_000_000;

/// Maximum number of activity entries per report.
pub const MAX_ACTIVITY_ENTRIES: usize = 100;

/// Native CLAW token ID (all zeros, represents the native token).
pub const NATIVE_TOKEN_ID: [u8; 32] = [0u8; 32];

/// Gas fee per transaction in native token units.
/// 0.001 CLAW = 1_000_000 units (assuming 9 decimals).
pub const GAS_FEE: u128 = 1_000_000;

/// CLAW token decimals.
pub const CLAW_DECIMALS: u8 = 9;

/// Total CLAW supply: 1 billion tokens = 1_000_000_000 * 10^9 base units.
pub const CLAW_TOTAL_SUPPLY: u128 = 1_000_000_000_000_000_000;

/// Native token symbol.
pub const NATIVE_TOKEN_SYMBOL: &str = "CLAW";
