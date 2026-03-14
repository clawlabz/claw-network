//! Wire protocol types for P2P messages.

use borsh::{BorshDeserialize, BorshSerialize};
use claw_types::block::Block;
use claw_types::transaction::Transaction;

/// Gossip topic names.
pub const TOPIC_TX: &str = "claw/tx/1";
pub const TOPIC_BLOCK: &str = "claw/block/1";
pub const TOPIC_VOTE: &str = "claw/vote/1";

/// Maximum P2P message size (1 MB).
pub const MAX_P2P_MESSAGE_SIZE: usize = 1024 * 1024;

/// Maximum number of simultaneous peer connections.
pub const MAX_PEER_CONNECTIONS: usize = 128;

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
    },
}
