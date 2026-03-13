//! Validator set management: staking, unstaking, epoch rotation.

use std::collections::BTreeMap;

use claw_types::state::ReputationAttestation;

use crate::types::*;

/// Manages the validator candidate pool and active validator set.
#[derive(Debug, Clone, Default)]
pub struct ValidatorSet {
    /// All staked candidates: address → StakeInfo.
    pub candidates: BTreeMap<[u8; 32], StakeInfo>,
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

    /// Create a validator set with initial stakes (for genesis).
    pub fn with_initial_stakes(stakes: Vec<([u8; 32], u128)>) -> Self {
        let mut vs = Self::new();
        for (addr, amount) in stakes {
            vs.candidates.insert(addr, StakeInfo {
                address: addr,
                amount,
                staked_at: 0,
            });
        }
        vs.recalculate_active(&[]);
        vs
    }

    /// Add or increase stake for a validator candidate.
    /// Returns Err if the resulting stake is below MIN_STAKE.
    pub fn stake(&mut self, address: [u8; 32], amount: u128, block_height: u64) -> Result<(), &'static str> {
        let current = self.candidates.get(&address).map(|s| s.amount).unwrap_or(0);
        let new_amount = current.checked_add(amount).ok_or("stake overflow")?;

        if new_amount < MIN_STAKE {
            return Err("stake below minimum");
        }

        let entry = self.candidates.entry(address).or_insert(StakeInfo {
            address,
            amount: 0,
            staked_at: block_height,
        });
        entry.amount = new_amount;
        entry.staked_at = block_height;
        Ok(())
    }

    /// Reduce or remove stake for a validator candidate.
    pub fn unstake(&mut self, address: &[u8; 32], amount: u128) -> Result<u128, &'static str> {
        let entry = self.candidates.get_mut(address).ok_or("not a candidate")?;
        if amount > entry.amount {
            return Err("unstake exceeds staked amount");
        }
        entry.amount -= amount;
        let remaining = entry.amount;

        // Remove candidate if below minimum
        if remaining < MIN_STAKE {
            self.candidates.remove(address);
        }
        Ok(remaining)
    }

    /// Recalculate the active validator set based on current stakes and reputation data.
    /// Called at epoch boundaries (every EPOCH_LENGTH blocks).
    pub fn recalculate_active(&mut self, reputation: &[ReputationAttestation]) {
        // Aggregate agent scores from reputation attestations
        let agent_scores = aggregate_agent_scores(reputation);

        // Calculate weights for all candidates
        let mut weighted: Vec<ActiveValidator> = self
            .candidates
            .values()
            .map(|stake_info| {
                let agent_score = agent_scores
                    .get(&stake_info.address)
                    .copied()
                    .unwrap_or(0);
                let weight = compute_weight(
                    stake_info.amount,
                    &self.candidates,
                    agent_score,
                    &agent_scores,
                    self.weight_config,
                );
                ActiveValidator {
                    address: stake_info.address,
                    stake: stake_info.amount,
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
/// Score = sum of positive attestation scores, clamped to [0, 10000].
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
/// Both components are normalized relative to the full candidate pool,
/// producing a weight in basis points (0–10000).
fn compute_weight(
    stake: u128,
    all_candidates: &BTreeMap<[u8; 32], StakeInfo>,
    agent_score: u64,
    all_scores: &BTreeMap<[u8; 32], u64>,
    config: WeightConfig,
) -> u64 {
    let total_stake: u128 = all_candidates.values().map(|s| s.amount).sum();
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
