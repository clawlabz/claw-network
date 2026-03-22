//! Slashing: equivocation detection and downtime penalties.
//!
//! All slashing operations modify WorldState.stakes directly (the single source
//! of truth). ValidatorSet is never mutated by slashing code.

use std::collections::BTreeMap;

use crate::types::{EPOCH_LENGTH, MIN_STAKE};

/// Penalty for equivocation (double-signing): 10% of stake (1000 basis points).
const EQUIVOCATION_SLASH_BPS: u64 = 1000;

/// Penalty for downtime (missed proposals): 1% of stake (100 basis points).
const DOWNTIME_SLASH_BPS: u64 = 100;

/// Threshold for downtime slashing: validator must miss > 50% of their slots.
const DOWNTIME_THRESHOLD_PERCENT: u64 = 50;

/// Jail duration in blocks (1 epoch).
const JAIL_DURATION: u64 = EPOCH_LENGTH;

/// Evidence that a validator signed two different blocks at the same height.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EquivocationEvidence {
    pub validator: [u8; 32],
    pub height: u64,
    pub block_hash_a: [u8; 32],
    pub signature_a: Vec<u8>,
    pub block_hash_b: [u8; 32],
    pub signature_b: Vec<u8>,
}

/// Tracks slashing state: jailed validators, equivocation evidence, and missed slots.
///
/// This state is persisted via WorldState fields (jailed_validators,
/// validator_missed_slots, validator_assigned_slots). The in-memory
/// SlashingState is reconstructed from WorldState on startup.
#[derive(Debug, Clone, Default)]
pub struct SlashingState {
    /// Jailed validators: address -> jail_until_height (exclusive).
    pub jailed: BTreeMap<[u8; 32], u64>,
    /// Equivocation evidence collected in the current epoch.
    pub evidence: Vec<EquivocationEvidence>,
    /// Missed proposal slots per validator in the current epoch.
    pub missed_slots: BTreeMap<[u8; 32], u64>,
    /// Total proposal slots assigned per validator in the current epoch.
    pub assigned_slots: BTreeMap<[u8; 32], u64>,
}

impl SlashingState {
    /// Create a new empty slashing state.
    pub fn new() -> Self {
        Self::default()
    }

    /// Check if a validator is currently jailed at the given height.
    pub fn is_jailed(&self, address: &[u8; 32], current_height: u64) -> bool {
        match self.jailed.get(address) {
            Some(&jail_until) => current_height < jail_until,
            None => false,
        }
    }

    /// Record equivocation evidence and apply the penalty.
    ///
    /// Slashes the validator's stake directly in `stakes` (WorldState.stakes).
    /// Returns the amount slashed, or an error if evidence is invalid.
    pub fn report_equivocation(
        &mut self,
        evidence: EquivocationEvidence,
        stakes: &mut BTreeMap<[u8; 32], u128>,
        current_height: u64,
    ) -> Result<u128, &'static str> {
        // Validate: must be two different block hashes
        if evidence.block_hash_a == evidence.block_hash_b {
            return Err("equivocation evidence has identical block hashes");
        }

        // Validate: signatures must be different
        if evidence.signature_a == evidence.signature_b {
            return Err("equivocation evidence has identical signatures");
        }

        // Check validator has an active stake
        if !stakes.contains_key(&evidence.validator) {
            return Err("validator has no active stake");
        }

        // Apply slash: 10% of stake
        let slashed = slash_stake(stakes, &evidence.validator, EQUIVOCATION_SLASH_BPS);

        // Jail the validator for 1 epoch
        let jail_until = current_height + JAIL_DURATION;
        self.jailed.insert(evidence.validator, jail_until);

        tracing::warn!(
            validator = %hex::encode(evidence.validator),
            height = evidence.height,
            slashed_amount = slashed,
            jail_until,
            "Equivocation detected — validator slashed and jailed"
        );

        // Store evidence
        self.evidence.push(evidence);

        Ok(slashed)
    }

    /// Record that a validator was elected as proposer for a slot.
    pub fn record_assigned_slot(&mut self, validator: &[u8; 32]) {
        *self.assigned_slots.entry(*validator).or_insert(0) += 1;
    }

    /// Record that a validator missed their proposal slot.
    pub fn record_missed_slot(&mut self, validator: &[u8; 32]) {
        *self.missed_slots.entry(*validator).or_insert(0) += 1;
    }

    /// Identify offline validators at epoch boundary.
    ///
    /// Validators that missed > 50% of their assigned proposal slots in this
    /// epoch are marked as offline. They are NOT slashed or jailed — they
    /// simply won't receive block rewards in the next epoch.
    ///
    /// Design rationale: downtime is not malicious behavior. Slashing is
    /// reserved for equivocation (double-signing). Withholding rewards is
    /// sufficient economic incentive to stay online. This aligns with
    /// Ethereum, Polkadot, and Solana's approach.
    ///
    /// Returns the list of offline validator addresses.
    pub fn process_downtime_penalties(&self) -> Vec<[u8; 32]> {
        let mut offline_validators = Vec::new();

        for (validator, &assigned) in &self.assigned_slots {
            if assigned == 0 {
                continue;
            }

            let missed = self.missed_slots.get(validator).copied().unwrap_or(0);
            let missed_percent = (missed * 100) / assigned;

            if missed_percent > DOWNTIME_THRESHOLD_PERCENT {
                tracing::info!(
                    validator = %hex::encode(validator),
                    missed,
                    assigned,
                    missed_percent,
                    "Validator offline — excluded from next epoch rewards (no slash)"
                );
                offline_validators.push(*validator);
            }
        }

        offline_validators
    }

    /// Reset per-epoch counters. Called at epoch boundary after processing.
    pub fn reset_epoch_counters(&mut self) {
        self.missed_slots.clear();
        self.assigned_slots.clear();
        self.evidence.clear();
    }

    /// Unjail validators whose jail period has expired.
    /// Returns the list of unjailed validator addresses.
    pub fn unjail_expired(&mut self, current_height: u64) -> Vec<[u8; 32]> {
        let expired: Vec<[u8; 32]> = self
            .jailed
            .iter()
            .filter(|(_, &jail_until)| current_height >= jail_until)
            .map(|(addr, _)| *addr)
            .collect();

        for addr in &expired {
            self.jailed.remove(addr);
            tracing::info!(
                validator = %hex::encode(addr),
                height = current_height,
                "Validator unjailed"
            );
        }

        expired
    }
}

/// Slash a validator's stake by the given basis points (e.g., 1000 = 10%).
/// Operates directly on WorldState.stakes. If the remaining stake falls below
/// MIN_STAKE, the stake entry is removed entirely.
/// Returns the amount slashed (burned — not credited to anyone).
fn slash_stake(
    stakes: &mut BTreeMap<[u8; 32], u128>,
    address: &[u8; 32],
    basis_points: u64,
) -> u128 {
    let stake = match stakes.get(address) {
        Some(&amount) => amount,
        None => return 0,
    };

    let slash_amount = (stake * basis_points as u128) / 10_000;
    if slash_amount == 0 {
        return 0;
    }

    let remaining = stake.saturating_sub(slash_amount);

    if remaining < MIN_STAKE {
        // Remove stake entirely
        stakes.remove(address);
    } else {
        // Reduce stake
        *stakes.get_mut(address).unwrap() = remaining;
    }

    slash_amount
}

#[cfg(test)]
mod tests {
    use super::*;
    use claw_crypto::ed25519_dalek::SigningKey;

    fn make_address(seed: u8) -> [u8; 32] {
        let sk = SigningKey::from_bytes(&[seed; 32]);
        sk.verifying_key().to_bytes()
    }

    fn setup_stakes() -> BTreeMap<[u8; 32], u128> {
        let mut stakes = BTreeMap::new();
        stakes.insert(make_address(1), MIN_STAKE * 10);
        stakes.insert(make_address(2), MIN_STAKE * 5);
        stakes.insert(make_address(3), MIN_STAKE * 2);
        stakes
    }

    #[test]
    fn jailing_and_unjailing() {
        let mut state = SlashingState::new();
        let addr = make_address(1);

        // Not jailed initially
        assert!(!state.is_jailed(&addr, 0));

        // Jail at height 50, until 50 + 100 = 150
        state.jailed.insert(addr, 150);
        assert!(state.is_jailed(&addr, 50));
        assert!(state.is_jailed(&addr, 149));
        assert!(!state.is_jailed(&addr, 150));
        assert!(!state.is_jailed(&addr, 200));

        // Reset jail
        state.jailed.insert(addr, 200);
        let unjailed = state.unjail_expired(150);
        assert!(unjailed.is_empty());

        let unjailed = state.unjail_expired(200);
        assert_eq!(unjailed.len(), 1);
        assert_eq!(unjailed[0], addr);
        assert!(!state.is_jailed(&addr, 200));
    }

    #[test]
    fn equivocation_slashes_10_percent() {
        let mut stakes = setup_stakes();
        let mut slashing = SlashingState::new();
        let addr = make_address(1);
        let original_stake = stakes[&addr]; // 10 * MIN_STAKE

        let evidence = EquivocationEvidence {
            validator: addr,
            height: 42,
            block_hash_a: [1u8; 32],
            signature_a: vec![1, 2, 3],
            block_hash_b: [2u8; 32],
            signature_b: vec![4, 5, 6],
        };

        let slashed = slashing
            .report_equivocation(evidence, &mut stakes, 50)
            .unwrap();
        let expected_slash = original_stake / 10; // 10%
        assert_eq!(slashed, expected_slash);
        assert_eq!(stakes[&addr], original_stake - expected_slash);

        // Validator should be jailed
        assert!(slashing.is_jailed(&addr, 50));
        assert!(slashing.is_jailed(&addr, 149));
        assert!(!slashing.is_jailed(&addr, 150));
    }

    #[test]
    fn equivocation_same_hash_rejected() {
        let mut stakes = setup_stakes();
        let mut slashing = SlashingState::new();

        let evidence = EquivocationEvidence {
            validator: make_address(1),
            height: 42,
            block_hash_a: [1u8; 32],
            signature_a: vec![1, 2, 3],
            block_hash_b: [1u8; 32], // same hash
            signature_b: vec![4, 5, 6],
        };

        assert!(slashing
            .report_equivocation(evidence, &mut stakes, 50)
            .is_err());
    }

    #[test]
    fn downtime_over_threshold_returns_offline_no_slash() {
        let stakes = setup_stakes();
        let mut slashing = SlashingState::new();
        let addr = make_address(2);
        let original_stake = stakes[&addr];

        // Assigned 10 slots, missed 6 (60% > 50%)
        for _ in 0..10 {
            slashing.record_assigned_slot(&addr);
        }
        for _ in 0..6 {
            slashing.record_missed_slot(&addr);
        }

        let offline = slashing.process_downtime_penalties();
        assert_eq!(offline.len(), 1);
        assert_eq!(offline[0], addr);

        // Stake should NOT be reduced (no slashing for downtime)
        assert_eq!(stakes[&addr], original_stake);

        // Validator should NOT be jailed (no jailing for downtime)
        assert!(!slashing.is_jailed(&addr, 100));
    }

    #[test]
    fn downtime_under_threshold_not_offline() {
        let mut slashing = SlashingState::new();
        let addr = make_address(2);

        // Assigned 10 slots, missed 4 (40% < 50%)
        for _ in 0..10 {
            slashing.record_assigned_slot(&addr);
        }
        for _ in 0..4 {
            slashing.record_missed_slot(&addr);
        }

        let offline = slashing.process_downtime_penalties();
        assert!(offline.is_empty());
    }

    #[test]
    fn slash_below_min_removes_stake() {
        let mut stakes = BTreeMap::new();
        let addr = make_address(1);
        stakes.insert(addr, MIN_STAKE); // exactly at minimum

        let mut slashing = SlashingState::new();

        let evidence = EquivocationEvidence {
            validator: addr,
            height: 1,
            block_hash_a: [1u8; 32],
            signature_a: vec![1],
            block_hash_b: [2u8; 32],
            signature_b: vec![2],
        };

        slashing
            .report_equivocation(evidence, &mut stakes, 10)
            .unwrap();
        // After 10% slash, remaining = 90% of MIN_STAKE < MIN_STAKE
        assert!(!stakes.contains_key(&addr));
    }

    #[test]
    fn reset_epoch_counters_clears_state() {
        let mut slashing = SlashingState::new();
        let addr = make_address(1);

        slashing.record_assigned_slot(&addr);
        slashing.record_missed_slot(&addr);
        slashing.evidence.push(EquivocationEvidence {
            validator: addr,
            height: 1,
            block_hash_a: [1u8; 32],
            signature_a: vec![1],
            block_hash_b: [2u8; 32],
            signature_b: vec![2],
        });

        slashing.reset_epoch_counters();
        assert!(slashing.missed_slots.is_empty());
        assert!(slashing.assigned_slots.is_empty());
        assert!(slashing.evidence.is_empty());
        // Jailed validators should NOT be cleared by epoch reset
    }
}
