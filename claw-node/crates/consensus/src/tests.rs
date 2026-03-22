//! Tests for the consensus engine.

use std::collections::BTreeMap;

use claw_crypto::ed25519_dalek::{Signer, SigningKey};
use claw_types::state::ReputationAttestation;

use crate::election::elect_proposer;
use crate::types::*;
use crate::validator_set::ValidatorSet;
use crate::voting::{VoteCollector, VoteResult};

fn make_address(seed: u8) -> [u8; 32] {
    let sk = SigningKey::from_bytes(&[seed; 32]);
    sk.verifying_key().to_bytes()
}

fn make_signing_key(seed: u8) -> SigningKey {
    SigningKey::from_bytes(&[seed; 32])
}

// ==================== WeightConfig ====================

#[test]
fn weight_config_cold_start() {
    let c = WeightConfig::COLD_START;
    assert_eq!(c.stake_bps, 7000);
    assert_eq!(c.score_bps, 3000);
    assert_eq!(c.stake_bps + c.score_bps, 10000);
}

#[test]
fn weight_config_target() {
    let c = WeightConfig::TARGET;
    assert_eq!(c.stake_bps, 4000);
    assert_eq!(c.score_bps, 6000);
}

// ==================== Quorum ====================

#[test]
fn quorum_values() {
    assert_eq!(quorum(1), 1);
    assert_eq!(quorum(3), 3);   // 2*3/3 + 1 = 3
    assert_eq!(quorum(4), 3);   // 2*4/3 + 1 = 3
    assert_eq!(quorum(7), 5);   // 2*7/3 + 1 = 5
    assert_eq!(quorum(21), 15); // 2*21/3 + 1 = 15
}

// ==================== ValidatorSet ====================

#[test]
fn recalculate_active_top_n() {
    // Create 25 staked validators (more than MAX_VALIDATORS=21)
    let mut stakes = BTreeMap::new();
    for i in 0..25u8 {
        let addr = make_address(i);
        let amount = MIN_STAKE + (i as u128 * MIN_STAKE);
        stakes.insert(addr, amount);
    }

    let mut vs = ValidatorSet::new();
    vs.recalculate_active(&stakes, &[], None, 0);
    assert_eq!(vs.active.len(), MAX_VALIDATORS);

    // Active set should be sorted by weight descending
    for i in 1..vs.active.len() {
        assert!(vs.active[i - 1].weight >= vs.active[i].weight);
    }
}

#[test]
fn recalculate_with_reputation() {
    let addr_high_stake = make_address(1);
    let addr_high_rep = make_address(2);

    let mut stakes = BTreeMap::new();
    // High stake, low reputation
    stakes.insert(addr_high_stake, MIN_STAKE * 10);
    // Low stake, high reputation
    stakes.insert(addr_high_rep, MIN_STAKE);

    // Give high reputation to addr_high_rep
    let attestations: Vec<ReputationAttestation> = (0..50)
        .map(|i| ReputationAttestation {
            from: make_address(100),
            to: addr_high_rep,
            category: "game".into(),
            score: 100,
            platform: "test".into(),
            memo: String::new(),
            block_height: i,
        })
        .collect();

    let mut vs = ValidatorSet::new();
    vs.recalculate_active(&stakes, &attestations, None, 0);
    assert_eq!(vs.active.len(), 2);

    // Both should have non-zero weight
    for v in &vs.active {
        assert!(v.weight > 0);
    }
}

#[test]
fn with_initial_stakes() {
    let mut stakes = BTreeMap::new();
    stakes.insert(make_address(1), MIN_STAKE * 5);
    stakes.insert(make_address(2), MIN_STAKE * 3);
    stakes.insert(make_address(3), MIN_STAKE * 1);

    let vs = ValidatorSet::with_initial_stakes(&stakes);
    assert_eq!(vs.active.len(), 3);
    assert_eq!(vs.epoch, 1);
}

#[test]
fn is_epoch_boundary() {
    assert!(!ValidatorSet::is_epoch_boundary(0));
    assert!(!ValidatorSet::is_epoch_boundary(1));
    assert!(!ValidatorSet::is_epoch_boundary(99));
    assert!(ValidatorSet::is_epoch_boundary(100));
    assert!(!ValidatorSet::is_epoch_boundary(101));
    assert!(ValidatorSet::is_epoch_boundary(200));
}

// ==================== Election ====================

#[test]
fn elect_proposer_empty() {
    assert!(elect_proposer(&[], &[0u8; 32], 1).is_none());
}

#[test]
fn elect_proposer_single() {
    let addr = make_address(1);
    let validators = vec![ActiveValidator {
        address: addr,
        stake: MIN_STAKE,
        agent_score: 0,
        weight: 100,
    }];
    let result = elect_proposer(&validators, &[0u8; 32], 1);
    assert_eq!(result, Some(addr));
}

#[test]
fn elect_proposer_deterministic() {
    let validators: Vec<ActiveValidator> = (0..5u8)
        .map(|i| ActiveValidator {
            address: make_address(i),
            stake: MIN_STAKE,
            agent_score: 0,
            weight: 100 + i as u64 * 50,
        })
        .collect();

    let prev_hash = [42u8; 32];

    // Same inputs → same result
    let r1 = elect_proposer(&validators, &prev_hash, 10);
    let r2 = elect_proposer(&validators, &prev_hash, 10);
    assert_eq!(r1, r2);

    // Different height → likely different result
    let r3 = elect_proposer(&validators, &prev_hash, 11);
    // (not guaranteed different, but the function should be deterministic for same inputs)
    let _ = r3;
}

#[test]
fn elect_proposer_weighted_distribution() {
    // One validator has 90% weight, should be selected most of the time
    let heavy = make_address(1);
    let light = make_address(2);
    let validators = vec![
        ActiveValidator {
            address: heavy,
            stake: MIN_STAKE * 9,
            agent_score: 0,
            weight: 9000,
        },
        ActiveValidator {
            address: light,
            stake: MIN_STAKE,
            agent_score: 0,
            weight: 1000,
        },
    ];

    let mut heavy_count = 0u32;
    for h in 0..1000u64 {
        if elect_proposer(&validators, &[0u8; 32], h) == Some(heavy) {
            heavy_count += 1;
        }
    }

    // Expect roughly 900 out of 1000, allow some variance
    assert!(heavy_count > 800, "heavy_count={heavy_count}, expected ~900");
    assert!(heavy_count < 980, "heavy_count={heavy_count}, expected ~900");
}

// ==================== Voting ====================

#[test]
fn vote_collector_basic_finality() {
    let keys: Vec<SigningKey> = (0..3u8).map(make_signing_key).collect();
    let validators: Vec<ActiveValidator> = keys
        .iter()
        .map(|k| ActiveValidator {
            address: k.verifying_key().to_bytes(),
            stake: MIN_STAKE,
            agent_score: 0,
            weight: 100,
        })
        .collect();

    let block_hash = [99u8; 32];
    let height = 42;
    let mut collector = VoteCollector::new(height, block_hash, validators);

    // quorum(3) = 3, need all 3

    // Vote 1
    let msg = Vote::signable_bytes(&block_hash, height);
    let sig = keys[0].sign(&msg);
    let vote = Vote {
        block_hash,
        height,
        voter: keys[0].verifying_key().to_bytes(),
        signature: sig.to_bytes(),
    };
    let result = collector.add_vote(vote).unwrap();
    assert_eq!(result, VoteResult::Pending { votes: 1, needed: 3 });

    // Vote 2
    let sig = keys[1].sign(&msg);
    let vote = Vote {
        block_hash,
        height,
        voter: keys[1].verifying_key().to_bytes(),
        signature: sig.to_bytes(),
    };
    let result = collector.add_vote(vote).unwrap();
    assert_eq!(result, VoteResult::Pending { votes: 2, needed: 3 });

    // Vote 3 — finalized
    let sig = keys[2].sign(&msg);
    let vote = Vote {
        block_hash,
        height,
        voter: keys[2].verifying_key().to_bytes(),
        signature: sig.to_bytes(),
    };
    let result = collector.add_vote(vote).unwrap();
    assert_eq!(result, VoteResult::Finalized { votes: 3 });
}

#[test]
fn vote_invalid_signature_rejected() {
    let key = make_signing_key(1);
    let validators = vec![ActiveValidator {
        address: key.verifying_key().to_bytes(),
        stake: MIN_STAKE,
        agent_score: 0,
        weight: 100,
    }];

    let block_hash = [99u8; 32];
    let mut collector = VoteCollector::new(1, block_hash, validators);

    let vote = Vote {
        block_hash,
        height: 1,
        voter: key.verifying_key().to_bytes(),
        signature: [0u8; 64], // bad signature
    };
    assert!(collector.add_vote(vote).is_err());
}

#[test]
fn vote_non_validator_rejected() {
    let key = make_signing_key(1);
    let outsider = make_signing_key(99);
    let validators = vec![ActiveValidator {
        address: key.verifying_key().to_bytes(),
        stake: MIN_STAKE,
        agent_score: 0,
        weight: 100,
    }];

    let block_hash = [99u8; 32];
    let mut collector = VoteCollector::new(1, block_hash, validators);

    let msg = Vote::signable_bytes(&block_hash, 1);
    let sig = outsider.sign(&msg);
    let vote = Vote {
        block_hash,
        height: 1,
        voter: outsider.verifying_key().to_bytes(),
        signature: sig.to_bytes(),
    };
    assert!(collector.add_vote(vote).is_err());
}

#[test]
fn vote_duplicate_ignored() {
    let key = make_signing_key(1);
    let validators = vec![ActiveValidator {
        address: key.verifying_key().to_bytes(),
        stake: MIN_STAKE,
        agent_score: 0,
        weight: 100,
    }];

    let block_hash = [99u8; 32];
    let mut collector = VoteCollector::new(1, block_hash, validators);

    let msg = Vote::signable_bytes(&block_hash, 1);
    let sig = key.sign(&msg);
    let vote = Vote {
        block_hash,
        height: 1,
        voter: key.verifying_key().to_bytes(),
        signature: sig.to_bytes(),
    };

    collector.add_vote(vote.clone()).unwrap();
    let result = collector.add_vote(vote).unwrap();
    assert_eq!(collector.vote_count(), 1);
    assert_eq!(result, VoteResult::Finalized { votes: 1 }); // quorum(1) = 1
}

#[test]
fn vote_with_4_validators_needs_3() {
    // quorum(4) = 3
    let keys: Vec<SigningKey> = (0..4u8).map(make_signing_key).collect();
    let validators: Vec<ActiveValidator> = keys
        .iter()
        .map(|k| ActiveValidator {
            address: k.verifying_key().to_bytes(),
            stake: MIN_STAKE,
            agent_score: 0,
            weight: 100,
        })
        .collect();

    let block_hash = [42u8; 32];
    let height = 10;
    let mut collector = VoteCollector::new(height, block_hash, validators);

    let msg = Vote::signable_bytes(&block_hash, height);

    // 2 votes — still pending
    for i in 0..2 {
        let sig = keys[i].sign(&msg);
        let vote = Vote {
            block_hash,
            height,
            voter: keys[i].verifying_key().to_bytes(),
            signature: sig.to_bytes(),
        };
        let result = collector.add_vote(vote).unwrap();
        assert!(matches!(result, VoteResult::Pending { .. }));
    }

    // 3rd vote — finalized
    let sig = keys[2].sign(&msg);
    let vote = Vote {
        block_hash,
        height,
        voter: keys[2].verifying_key().to_bytes(),
        signature: sig.to_bytes(),
    };
    let result = collector.add_vote(vote).unwrap();
    assert_eq!(result, VoteResult::Finalized { votes: 3 });
}
