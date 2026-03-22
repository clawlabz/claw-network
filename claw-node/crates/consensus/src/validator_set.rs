//! Validator set management: active set recalculation from WorldState stakes.
//!
//! The ValidatorSet no longer maintains its own candidates — WorldState.stakes
//! is the single source of truth. This struct only tracks the computed active
//! validator set and epoch metadata.

use std::collections::BTreeMap;

use claw_types::state::ReputationAttestation;

use crate::slashing::SlashingState;
use crate::types::*;

/// Manages the active validator set (computed from WorldState.stakes each epoch).
#[derive(Debug, Clone, Default)]
pub struct ValidatorSet {
    /// Current active validators (top N by weight, recalculated each epoch).
    pub active: Vec<ActiveValidator>,
    /// Current epoch number.
    pub epoch: u64,
    /// Weight config for current epoch.
    pub weight_config: WeightConfig,
}

impl ValidatorSet {
    /// Create a new empty validator set.
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a validator set from initial stakes (for genesis).
    /// Reads directly from the provided stakes map (WorldState.stakes).
    pub fn with_initial_stakes(stakes: &BTreeMap<[u8; 32], u128>) -> Self {
        let mut vs = Self::new();
        vs.recalculate_active(stakes, &[], None, 0);
        vs
    }

    /// Recalculate the active validator set from WorldState.stakes.
    /// Called at epoch boundaries (every EPOCH_LENGTH blocks).
    pub fn recalculate_active(
        &mut self,
        stakes: &BTreeMap<[u8; 32], u128>,
        reputation: &[ReputationAttestation],
        slashing: Option<&SlashingState>,
        current_height: u64,
    ) {
        // Aggregate agent scores from reputation attestations
        let agent_scores = aggregate_agent_scores(reputation);

        // Calculate weights for all staked validators, filtering out jailed ones
        let mut weighted: Vec<ActiveValidator> = stakes
            .iter()
            .filter(|(_, &amount)| amount >= MIN_STAKE)
            .filter(|(addr, _)| {
                // Exclude jailed validators from the active set
                match slashing {
                    Some(s) => !s.is_jailed(addr, current_height),
                    None => true,
                }
            })
            .map(|(addr, &amount)| {
                let agent_score = agent_scores.get(addr).copied().unwrap_or(0);
                let weight = compute_weight(
                    amount,
                    stakes,
                    agent_score,
                    &agent_scores,
                    self.weight_config,
                );
                ActiveValidator {
                    address: *addr,
                    stake: amount,
                    agent_score,
                    weight,
                }
            })
            .collect();

        // Sort by weight descending, then by address for determinism
        weighted.sort_by(|a, b| {
            b.weight
                .cmp(&a.weight)
                .then_with(|| a.address.cmp(&b.address))
        });

        // Take top MAX_VALIDATORS
        weighted.truncate(MAX_VALIDATORS);
        self.active = weighted;

        // Ensure at least one validator remains active
        if self.active.is_empty() && !stakes.is_empty() {
            // If all were jailed, pick the one with highest stake regardless of jail status
            if let Some((addr, &amount)) = stakes.iter().rev().next() {
                self.active.push(ActiveValidator {
                    address: *addr,
                    stake: amount,
                    agent_score: 0,
                    weight: 1,
                });
                tracing::warn!("All validators jailed — keeping last resort validator to prevent chain halt");
            }
        }

        self.epoch += 1;
    }

    /// Check if a given height is an epoch boundary.
    pub fn is_epoch_boundary(height: u64) -> bool {
        height > 0 && height % EPOCH_LENGTH == 0
    }

    /// Get an active validator by address.
    pub fn get_active(&self, address: &[u8; 32]) -> Option<&ActiveValidator> {
        self.active.iter().find(|v| &v.address == address)
    }

    /// Check if an address is an active validator.
    pub fn is_active(&self, address: &[u8; 32]) -> bool {
        self.active.iter().any(|v| &v.address == address)
    }

    /// Total weight of all active validators.
    pub fn total_weight(&self) -> u64 {
        self.active.iter().map(|v| v.weight).sum()
    }

    /// Get the current epoch info snapshot.
    pub fn epoch_info(&self) -> EpochInfo {
        EpochInfo {
            epoch: self.epoch,
            validators: self.active.clone(),
            weight_config: self.weight_config,
        }
    }
}

/// Aggregate reputation attestations into per-address agent scores.
///
/// DEPRECATED: This function uses the legacy ReputationAttest-based scoring.
/// It is kept for backward compatibility during the transition to the new
/// multi-dimensional Agent Score system (see `claw_state::score`).
fn aggregate_agent_scores(reputation: &[ReputationAttestation]) -> BTreeMap<[u8; 32], u64> {
    let mut scores: BTreeMap<[u8; 32], i64> = BTreeMap::new();
    for att in reputation {
        *scores.entry(att.to).or_insert(0) += att.score as i64;
    }
    scores
        .into_iter()
        .map(|(addr, score)| (addr, score.clamp(0, 10000) as u64))
        .collect()
}

/// Compute the consensus weight for a single validator.
///
/// weight = normalize(stake) × stake_bps + normalize(agent_score) × score_bps
///
/// Both components are normalized relative to the full staked pool,
/// producing a weight in basis points (0–10000).
fn compute_weight(
    stake: u128,
    all_stakes: &BTreeMap<[u8; 32], u128>,
    agent_score: u64,
    all_scores: &BTreeMap<[u8; 32], u64>,
    config: WeightConfig,
) -> u64 {
    let total_stake: u128 = all_stakes.values().sum();
    let max_score: u64 = all_scores.values().copied().max().unwrap_or(1).max(1);

    // Normalized stake (0–10000 bps)
    let norm_stake = if total_stake > 0 {
        ((stake as u128 * 10000) / total_stake) as u64
    } else {
        0
    };

    // Normalized agent score (0–10000 bps)
    let norm_score = (agent_score * 10000) / max_score;

    // Weighted sum
    let w = (norm_stake * config.stake_bps as u64 + norm_score * config.score_bps as u64) / 10000;
    w.max(1) // minimum weight of 1 to ensure all active validators have some chance
}
