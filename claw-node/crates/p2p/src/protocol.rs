//! Wire protocol types for P2P messages.

use borsh::{BorshDeserialize, BorshSerialize};
use claw_types::block::Block;
use claw_types::transaction::Transaction;

/// Gossip topic names.
pub const TOPIC_TX: &str = "claw/tx/1";
pub const TOPIC_BLOCK: &str = "claw/block/1";

/// Maximum P2P message size (1 MB).
pub const MAX_P2P_MESSAGE_SIZE: usize = 1024 * 1024;

/// Maximum number of simultaneous peer connections.
pub const MAX_PEER_CONNECTIONS: usize = 128;

/// Gossip message types.
#[derive(Debug, Clone, BorshSerialize, BorshDeserialize)]
pub enum GossipMessage {
    /// A new transaction to propagate.
    NewTx(Transaction),
    /// A new block announcement (full block for simplicity in MVP).
    NewBlock(Block),
}

/// Sync request types.
#[derive(Debug, Clone, BorshSerialize, BorshDeserialize)]
pub enum SyncRequest {
    /// Request blocks from `from_height`, up to `count` blocks.
    GetBlocks { from_height: u64, count: u32 },
    /// Request peer's current chain height.
    GetStatus,
}

/// Sync response types.
#[derive(Debug, Clone, BorshSerialize, BorshDeserialize)]
pub enum SyncResponse {
    /// Blocks response.
    Blocks(Vec<Block>),
    /// Status response: current height.
    Status { height: u64 },
}
