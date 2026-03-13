//! BFT vote collection and finality.

use std::collections::BTreeMap;

use claw_crypto::ed25519_dalek::{Signature, VerifyingKey};

use crate::types::*;

/// Result of collecting votes for a block.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VoteResult {
    /// Not enough votes yet — still collecting.
    Pending { votes: usize, needed: usize },
    /// Quorum reached — block is finalized.
    Finalized { votes: usize },
    /// Conflicting votes detected (byzantine behavior).
    Conflict { voter: [u8; 32] },
}

/// Collects and verifies votes for a single block height.
pub struct VoteCollector {
    /// The block height being voted on.
    height: u64,
    /// Expected block hash (from the proposer).
    expected_hash: [u8; 32],
    /// Active validator set for this round.
    active_validators: Vec<ActiveValidator>,
    /// Collected valid votes: voter address → Vote.
    votes: BTreeMap<[u8; 32], Vote>,
}

impl VoteCollector {
    /// Create a new vote collector for a proposed block.
    pub fn new(
        height: u64,
        expected_hash: [u8; 32],
        active_validators: Vec<ActiveValidator>,
    ) -> Self {
        Self {
            height,
            expected_hash,
            active_validators,
            votes: BTreeMap::new(),
        }
    }

    /// Submit a vote. Returns the current vote result.
    pub fn add_vote(&mut self, vote: Vote) -> Result<VoteResult, &'static str> {
        // 1. Check voter is an active validator
        if !self.active_validators.iter().any(|v| v.address == vote.voter) {
            return Err("voter is not an active validator");
        }

        // 2. Check height matches
        if vote.height != self.height {
            return Err("vote height mismatch");
        }

        // 3. Check for conflicting vote (same voter, different hash)
        if let Some(existing) = self.votes.get(&vote.voter) {
            if existing.block_hash != vote.block_hash {
                return Ok(VoteResult::Conflict { voter: vote.voter });
            }
            // Duplicate vote for same hash — ignore
            return Ok(self.current_result());
        }

        // 4. Verify signature
        let msg = Vote::signable_bytes(&vote.block_hash, vote.height);
        let vk = VerifyingKey::from_bytes(&vote.voter)
            .map_err(|_| "invalid voter public key")?;
        let sig = Signature::from_bytes(&vote.signature);
        vk.verify_strict(&msg, &sig)
            .map_err(|_| "invalid vote signature")?;

        // 5. Check vote is for expected hash
        if vote.block_hash != self.expected_hash {
            // Valid signature but wrong hash — could be a fork attempt.
            // In our simplified BFT, we just reject it.
            return Err("vote for unexpected block hash");
        }

        // 6. Accept vote
        self.votes.insert(vote.voter, vote);
        Ok(self.current_result())
    }

    /// Get the current vote tally result.
    pub fn current_result(&self) -> VoteResult {
        let n = self.active_validators.len();
        let needed = quorum(n);
        let have = self.votes.len();

        if have >= needed {
            VoteResult::Finalized { votes: have }
        } else {
            VoteResult::Pending {
                votes: have,
                needed,
            }
        }
    }

    /// Number of votes collected so far.
    pub fn vote_count(&self) -> usize {
        self.votes.len()
    }

    /// The expected block hash.
    pub fn block_hash(&self) -> &[u8; 32] {
        &self.expected_hash
    }

    /// The block height.
    pub fn height(&self) -> u64 {
        self.height
    }
}
