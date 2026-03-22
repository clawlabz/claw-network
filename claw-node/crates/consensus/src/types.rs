//! Consensus types: staking, validator info, votes, epochs.

use borsh::{BorshDeserialize, BorshSerialize};
use serde::{Deserialize, Serialize};

/// Minimum stake to become a validator candidate (10,000 CLAW = 10_000 * 10^9).
pub const MIN_STAKE: u128 = 10_000_000_000_000;

/// Maximum active validators per epoch.
pub const MAX_VALIDATORS: usize = 21;

/// Blocks per epoch — validator set recalculated at each epoch boundary.
pub const EPOCH_LENGTH: u64 = 100;

/// Block time in seconds.
pub const BLOCK_TIME_SECS: u64 = 3;

/// BFT quorum: need > 2/3 of active validators to finalize a block.
/// For n validators, quorum = floor(2*n/3) + 1.
pub fn quorum(n: usize) -> usize {
    if n == 0 {
        return 0;
    }
    (2 * n / 3) + 1
}

/// Consensus weight ratios. During cold start, stake is weighted higher.
#[derive(Debug, Clone, Copy, BorshSerialize, BorshDeserialize, Serialize, Deserialize)]
pub struct WeightConfig {
    /// Stake weight ratio (0.0 to 1.0, stored as basis points 0–10000).
    pub stake_bps: u16,
    /// Agent score weight ratio (0.0 to 1.0, stored as basis points 0–10000).
    pub score_bps: u16,
}

impl WeightConfig {
    /// Cold-start config: 70% stake, 30% agent score.
    pub const COLD_START: Self = Self {
        stake_bps: 7000,
        score_bps: 3000,
    };

    /// Target config: 40% stake, 60% agent score.
    pub const TARGET: Self = Self {
        stake_bps: 4000,
        score_bps: 6000,
    };
}

impl Default for WeightConfig {
    fn default() -> Self {
        Self::COLD_START
    }
}

/// Information about a staked validator candidate.
#[derive(Debug, Clone, PartialEq, Eq, BorshSerialize, BorshDeserialize, Serialize, Deserialize)]
pub struct StakeInfo {
    /// Validator address (Ed25519 public key).
    pub address: [u8; 32],
    /// Amount of CLAW staked (in base units, 9 decimals).
    pub amount: u128,
    /// Block height at which the stake was last updated.
    pub staked_at: u64,
}

/// An active validator with computed weight.
#[derive(Debug, Clone, PartialEq, Eq, BorshSerialize, BorshDeserialize, Serialize, Deserialize)]
pub struct ActiveValidator {
    /// Validator address.
    pub address: [u8; 32],
    /// Staked amount.
    pub stake: u128,
    /// Aggregated agent reputation score (clamped to 0..=10000).
    pub agent_score: u64,
    /// Final computed weight (basis points, higher = more likely to propose).
    pub weight: u64,
}

/// A vote on a proposed block.
#[derive(Debug, Clone, PartialEq, Eq, BorshSerialize, BorshDeserialize)]
pub struct Vote {
    /// The block hash being voted on.
    pub block_hash: [u8; 32],
    /// The block height.
    pub height: u64,
    /// Voter's address.
    pub voter: [u8; 32],
    /// Ed25519 signature over (block_hash || height_le_bytes).
    pub signature: [u8; 64],
}

impl Vote {
    /// The bytes that are signed for a vote.
    pub fn signable_bytes(block_hash: &[u8; 32], height: u64) -> Vec<u8> {
        let mut buf = Vec::with_capacity(40);
        buf.extend_from_slice(block_hash);
        buf.extend_from_slice(&height.to_le_bytes());
        buf
    }
}

/// Epoch info snapshot.
#[derive(Debug, Clone, BorshSerialize, BorshDeserialize, Serialize, Deserialize)]
pub struct EpochInfo {
    /// Epoch number (height / EPOCH_LENGTH).
    pub epoch: u64,
    /// Active validator set for this epoch.
    pub validators: Vec<ActiveValidator>,
    /// Weight config used for this epoch.
    pub weight_config: WeightConfig,
}
