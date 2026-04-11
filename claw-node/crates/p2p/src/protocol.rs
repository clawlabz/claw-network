//! Wire protocol types for P2P messages.

use borsh::{BorshDeserialize, BorshSerialize};
use claw_types::block::Block;
use claw_types::state::MinerCheckinWitness;
use claw_types::transaction::Transaction;

/// Maximum P2P message size (1 MB).
pub const MAX_P2P_MESSAGE_SIZE: usize = 1024 * 1024;

/// Maximum number of simultaneous peer connections.
pub const MAX_PEER_CONNECTIONS: usize = 128;

/// Generate chain-id-scoped gossip topic names.
/// This ensures mainnet and testnet nodes on the same local network
/// do not exchange messages via gossipsub.
pub fn topic_tx(chain_id: &str) -> String {
    format!("claw/{}/tx/1", chain_id)
}

pub fn topic_block(chain_id: &str) -> String {
    format!("claw/{}/block/1", chain_id)
}

pub fn topic_vote(chain_id: &str) -> String {
    format!("claw/{}/vote/1", chain_id)
}

pub fn topic_miner_checkin(chain_id: &str) -> String {
    format!("claw/{}/miner-checkin/1", chain_id)
}

/// Generate chain-id-scoped sync protocol string.
/// This ensures request_response sync only connects to same-chain peers.
pub fn sync_protocol(chain_id: &str) -> String {
    format!("/claw/{}/sync/1", chain_id)
}

/// A BFT vote: a validator's signature on a block hash.
#[derive(Debug, Clone, BorshSerialize, BorshDeserialize)]
pub struct BlockVote {
    /// Block hash being voted on.
    pub block_hash: [u8; 32],
    /// Block height.
    pub height: u64,
    /// Voter's address (Ed25519 public key).
    pub voter: [u8; 32],
    /// Ed25519 signature over the block hash.
    pub signature: [u8; 64],
}

/// Gossip message types.
#[derive(Debug, Clone, BorshSerialize, BorshDeserialize)]
pub enum GossipMessage {
    /// A new transaction to propagate.
    NewTx(Transaction),
    /// A new block announcement (full block for simplicity in MVP).
    NewBlock(Block),
    /// A BFT vote on a block for finality.
    Vote(BlockVote),
    /// A miner checkin witness (V3 heartbeat replacement).
    MinerCheckin(MinerCheckinWitness),
}

/// Sync request types.
#[derive(Debug, Clone, BorshSerialize, BorshDeserialize)]
pub enum SyncRequest {
    /// Request blocks from `from_height`, up to `count` blocks.
    GetBlocks { from_height: u64, count: u32 },
    /// Request peer's current chain height.
    GetStatus,
    /// Request the latest state snapshot for fast sync.
    GetStateSnapshot,
    /// Fallback: push a miner checkin witness when gossipsub publish fails.
    PushMinerCheckin(MinerCheckinWitness),
}

/// Sync response types.
#[derive(Debug, Clone, BorshSerialize, BorshDeserialize)]
pub enum SyncResponse {
    /// Blocks response.
    Blocks(Vec<Block>),
    /// Status response: current height.
    Status { height: u64 },
    /// State snapshot response: latest state + height + state_root for fast sync.
    StateSnapshot {
        /// Block height at which the snapshot was taken.
        height: u64,
        /// State root hash for verification.
        state_root: [u8; 32],
        /// Borsh-serialized WorldState data.
        state_data: Vec<u8>,
        /// The latest block at snapshot height, used to re-establish chain continuity after fork recovery.
        latest_block: Block,
        /// Genesis block hash — receiver verifies this matches its own genesis.
        genesis_hash: [u8; 32],
    },
    /// ACK for PushMinerCheckin — pure acknowledgement, does not trigger sync.
    CheckinAccepted,
}

#[cfg(test)]
mod tests {
    use super::*;
    use borsh::{BorshDeserialize, BorshSerialize};

    /// Old SyncRequest enum (v0.5.7) — 3 variants only.
    /// Used to verify that old nodes reject new variant index 3 with Err, not panic.
    #[derive(Debug, Clone, BorshSerialize, BorshDeserialize)]
    enum SyncRequestV057 {
        GetBlocks { from_height: u64, count: u32 },
        GetStatus,
        GetStateSnapshot,
    }

    /// Old SyncResponse enum (v0.5.7) — 3 variants only.
    #[derive(Debug, Clone, BorshSerialize, BorshDeserialize)]
    enum SyncResponseV057 {
        Blocks(Vec<Block>),
        Status { height: u64 },
        StateSnapshot {
            height: u64,
            state_root: [u8; 32],
            state_data: Vec<u8>,
            latest_block: Block,
            genesis_hash: [u8; 32],
        },
    }

    #[test]
    fn old_sync_request_rejects_new_push_miner_checkin() {
        // Serialize the NEW variant (index 3)
        let witness = MinerCheckinWitness {
            miner: [0u8; 32],
            epoch: 100,
            ref_block_hash: [0u8; 32],
            ref_block_height: 1000,
            signature: [0u8; 64],
        };
        let new_bytes = borsh::to_vec(&SyncRequest::PushMinerCheckin(witness)).unwrap();
        assert_eq!(new_bytes[0], 3, "PushMinerCheckin should be variant index 3");

        // Old enum (3 variants) must reject index 3 with Err, not panic
        let old_result = SyncRequestV057::try_from_slice(&new_bytes);
        assert!(old_result.is_err(), "Old SyncRequest must reject unknown variant 3");

        // New enum must round-trip successfully
        let new_result = SyncRequest::try_from_slice(&new_bytes);
        assert!(new_result.is_ok(), "New SyncRequest must deserialize its own variant");
    }

    #[test]
    fn old_sync_response_rejects_new_checkin_accepted() {
        // Serialize the NEW variant (index 3)
        let new_bytes = borsh::to_vec(&SyncResponse::CheckinAccepted).unwrap();
        assert_eq!(new_bytes[0], 3, "CheckinAccepted should be variant index 3");

        // Old enum (3 variants) must reject index 3 with Err, not panic
        let old_result = SyncResponseV057::try_from_slice(&new_bytes);
        assert!(old_result.is_err(), "Old SyncResponse must reject unknown variant 3");

        // New enum must round-trip successfully
        let new_result = SyncResponse::try_from_slice(&new_bytes);
        assert!(new_result.is_ok(), "New SyncResponse must deserialize its own variant");
    }

    #[test]
    fn old_variants_still_round_trip_with_new_enum() {
        // Old GetStatus serialized → new enum can still deserialize
        let old_bytes = borsh::to_vec(&SyncRequestV057::GetStatus).unwrap();
        assert!(SyncRequest::try_from_slice(&old_bytes).is_ok());

        // Old Status response serialized → new enum can still deserialize
        let old_resp = borsh::to_vec(&SyncResponseV057::Status { height: 42 }).unwrap();
        assert!(SyncResponse::try_from_slice(&old_resp).is_ok());
    }
}
