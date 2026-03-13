//! Block proposer election using VRF-like weighted random selection.

use crate::types::ActiveValidator;

/// Elect a block proposer from the active validator set.
///
/// Uses a deterministic pseudo-random selection based on `prev_hash` and `height`,
/// weighted by each validator's consensus weight.
///
/// The "VRF" is simplified: `blake3(prev_hash || height_le_bytes)` produces a
/// seed, which is mapped into the cumulative weight range to pick a proposer.
/// This is deterministic and verifiable by any node with the same inputs.
pub fn elect_proposer(
    validators: &[ActiveValidator],
    prev_hash: &[u8; 32],
    height: u64,
) -> Option<[u8; 32]> {
    if validators.is_empty() {
        return None;
    }

    // Single validator — no election needed
    if validators.len() == 1 {
        return Some(validators[0].address);
    }

    // Compute VRF seed
    let seed = vrf_seed(prev_hash, height);

    // Total weight
    let total_weight: u64 = validators.iter().map(|v| v.weight).sum();
    if total_weight == 0 {
        return Some(validators[0].address);
    }

    // Map seed to [0, total_weight)
    let seed_u64 = u64::from_le_bytes(seed[..8].try_into().unwrap());
    let pick = seed_u64 % total_weight;

    // Walk cumulative weights to find the selected proposer
    let mut cumulative = 0u64;
    for v in validators {
        cumulative += v.weight;
        if pick < cumulative {
            return Some(v.address);
        }
    }

    // Fallback (shouldn't happen)
    Some(validators.last().unwrap().address)
}

/// Compute the VRF seed from previous block hash and height.
/// This is a deterministic function that any node can verify.
pub fn vrf_seed(prev_hash: &[u8; 32], height: u64) -> [u8; 32] {
    let mut input = Vec::with_capacity(40);
    input.extend_from_slice(prev_hash);
    input.extend_from_slice(&height.to_le_bytes());
    *blake3::hash(&input).as_bytes()
}
