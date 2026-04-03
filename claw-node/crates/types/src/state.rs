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

// --- Agent Mining types and constants ---

/// Miner tier classification.
#[derive(Debug, Clone, Copy, PartialEq, Eq, BorshSerialize, BorshDeserialize, Serialize, Deserialize)]
#[borsh(use_discriminant = true)]
#[repr(u8)]
pub enum MinerTier {
    /// Basic online miner.
    Online = 1,
}

/// Legacy V1 miner info — used only for deserializing pre-V2 snapshots.
#[derive(Debug, Clone, PartialEq, Eq, BorshDeserialize)]
pub struct MinerInfoV1 {
    pub address: [u8; 32],
    pub tier: MinerTier,
    pub name: String,
    pub registered_at: u64,
    pub last_heartbeat: u64,
    pub ip_prefix: Vec<u8>,
    pub active: bool,
    pub reputation_bps: u16,
}

/// On-chain state for a registered miner (V2: epoch scoring + delayed settlement).
#[derive(Debug, Clone, PartialEq, Eq, BorshSerialize, BorshDeserialize, Serialize, Deserialize)]
pub struct MinerInfo {
    /// Miner's Ed25519 public key / address.
    pub address: [u8; 32],
    /// Miner tier.
    pub tier: MinerTier,
    /// Human-readable name.
    pub name: String,
    /// Block height at which the miner was registered.
    pub registered_at: u64,
    /// Block height of the last heartbeat.
    pub last_heartbeat: u64,
    /// IP address prefix (first 3 bytes for /24 subnet check).
    pub ip_prefix: Vec<u8>,
    /// Whether the miner is currently active.
    pub active: bool,
    /// Reputation score in basis points (0-10000).
    pub reputation_bps: u16,
    // --- V2 fields (heartbeat redesign) ---
    /// Pending rewards awaiting confirmation by next epoch's heartbeat.
    pub pending_rewards: u128,
    /// Epoch number that pending_rewards corresponds to.
    pub pending_epoch: u64,
    /// Bitmap of attendance for the last 16 epochs (LSB = most recent).
    pub epoch_attendance: u16,
    /// Number of consecutive epochs missed.
    pub consecutive_misses: u16,
}

impl MinerInfo {
    /// Serialize only V1 fields for backward-compatible state_root computation.
    /// Used before HEARTBEAT_V2_HEIGHT to produce the same hash as old nodes.
    pub fn borsh_v1(&self) -> Vec<u8> {
        let mut buf = Vec::new();
        BorshSerialize::serialize(&self.address, &mut buf).unwrap();
        BorshSerialize::serialize(&self.tier, &mut buf).unwrap();
        BorshSerialize::serialize(&self.name, &mut buf).unwrap();
        BorshSerialize::serialize(&self.registered_at, &mut buf).unwrap();
        BorshSerialize::serialize(&self.last_heartbeat, &mut buf).unwrap();
        BorshSerialize::serialize(&self.ip_prefix, &mut buf).unwrap();
        BorshSerialize::serialize(&self.active, &mut buf).unwrap();
        BorshSerialize::serialize(&self.reputation_bps, &mut buf).unwrap();
        buf
    }
}

impl From<MinerInfoV1> for MinerInfo {
    fn from(v1: MinerInfoV1) -> Self {
        // V2 field values here are irrelevant — they will be overwritten by
        // the normalization step at HEARTBEAT_V2_HEIGHT. Use simple defaults.
        Self {
            address: v1.address,
            tier: v1.tier,
            name: v1.name,
            registered_at: v1.registered_at,
            last_heartbeat: v1.last_heartbeat,
            ip_prefix: v1.ip_prefix,
            active: v1.active,
            reputation_bps: v1.reputation_bps,
            pending_rewards: 0,
            pending_epoch: 0,
            epoch_attendance: 0,
            consecutive_misses: 0,
        }
    }
}

/// Legacy heartbeat interval in blocks (used before HEARTBEAT_V2_HEIGHT).
pub const MINER_HEARTBEAT_INTERVAL_V1: u64 = 1_000;

/// V2 heartbeat interval: one heartbeat per epoch.
pub const MINER_HEARTBEAT_INTERVAL: u64 = 100;

/// Maximum number of miners allowed per /24 subnet.
pub const MAX_MINERS_PER_SUBNET: usize = 3;

/// Legacy grace period in blocks (used before HEARTBEAT_V2_HEIGHT).
pub const MINER_GRACE_BLOCKS: u64 = 2_000;

/// Miner epoch length in blocks (~5 minutes at 3s block time).
pub const MINER_EPOCH_LENGTH: u64 = 100;

/// Number of past epochs to evaluate for uptime scoring.
pub const MINER_UPTIME_WINDOW: u32 = 12;

/// Minimum epochs with attendance (out of MINER_UPTIME_WINDOW) to qualify for rewards.
pub const MINER_MIN_UPTIME_FOR_REWARD: u32 = 6;

/// Consecutive missed epochs before a miner is deactivated.
pub const MINER_GRACE_EPOCHS: u16 = 6;

/// Reputation decay per missed epoch in basis points (1% = 100 bps).
pub const REPUTATION_DECAY_BPS: u16 = 100;

/// Block height at which Heartbeat V2 logic activates.
/// Before this height, legacy heartbeat interval and grace period apply.
/// MUST be divisible by MINER_EPOCH_LENGTH — all V2 logic activates atomically
/// at this epoch boundary (normalization + state_root switch + heartbeat mode).
pub const HEARTBEAT_V2_HEIGHT: u64 = 213_800; // TESTNET ONLY — change to mainnet value before release

// Compile-time check: V2_HEIGHT must be aligned to epoch boundary.
const _: () = assert!(HEARTBEAT_V2_HEIGHT % MINER_EPOCH_LENGTH == 0, "HEARTBEAT_V2_HEIGHT must be a multiple of MINER_EPOCH_LENGTH");

/// Reputation: newcomer tier (0-7 days), 20% reward weight.
pub const REPUTATION_NEWCOMER_BPS: u16 = 2_000;

/// Reputation: established tier (7-30 days), 50% reward weight.
pub const REPUTATION_ESTABLISHED_BPS: u16 = 5_000;

/// Reputation: veteran tier (30+ days), 100% reward weight.
pub const REPUTATION_VETERAN_BPS: u16 = 10_000;

/// Number of blocks in 7 days at 3-second block time (7 * 24 * 3600 / 3).
pub const BLOCKS_7_DAYS: u64 = 201_600;

/// Number of blocks in 30 days at 3-second block time (30 * 24 * 3600 / 3).
pub const BLOCKS_30_DAYS: u64 = 864_000;

/// Uptime tier multiplier (out of 100) based on epoch attendance count.
pub fn miner_uptime_multiplier(attendance_count: u32) -> u128 {
    match attendance_count {
        12 => 100,     // 1.0x — perfect attendance
        9..=11 => 80,  // 0.8x — occasional disconnects
        6..=8 => 50,   // 0.5x — unstable
        _ => 0,        // below 50% — no rewards
    }
}
