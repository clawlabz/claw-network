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
}
