//! Sync mode logic: Full, Fast, and Light node support.
//!
//! - **Full**: Download and store all blocks (default behavior).
//! - **Fast**: Download latest state snapshot from a peer, verify state_root,
//!   then sync only recent blocks.
//! - **Light**: Prune blocks older than N, keep only state + recent blocks.

use std::path::Path;

use claw_storage::ChainStore;
use claw_types::Block;

use crate::chain::Chain;

/// Default number of recent blocks to retain in Light mode.
pub const LIGHT_MODE_KEEP_BLOCKS: u64 = 1_000;

/// Pruning check interval in seconds.
const PRUNE_CHECK_INTERVAL_SECS: u64 = 30;

/// Sync mode for the node.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SyncMode {
    /// Download and store every block from genesis.
    Full,
    /// Download a state snapshot from peers, then sync only recent blocks.
    Fast,
    /// Like Full, but periodically prune old blocks to save disk space.
    Light,
}

impl SyncMode {
    /// Parse a sync mode from a CLI string. Defaults to `Full` for unknown values.
    pub fn parse(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "fast" => SyncMode::Fast,
            "light" => SyncMode::Light,
            _ => SyncMode::Full,
        }
    }
}

impl std::fmt::Display for SyncMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SyncMode::Full => write!(f, "full"),
            SyncMode::Fast => write!(f, "fast"),
            SyncMode::Light => write!(f, "light"),
        }
    }
}

/// For Light Node mode: prune old blocks from storage, keeping the most recent
/// `keep_blocks` blocks. Genesis (height 0) is always preserved.
pub fn prune_old_blocks(store: &ChainStore, current_height: u64, keep_blocks: u64) {
    let prune_below = current_height.saturating_sub(keep_blocks);
    if prune_below <= 1 {
        return;
    }

    let pruned = store.prune_blocks_below(prune_below);
    if pruned > 0 {
        tracing::info!(
            pruned_count = pruned,
            below_height = prune_below,
            current_height,
            keep_blocks,
            "Light node: pruned old blocks"
        );
    }
}

/// Run a periodic pruning loop for Light mode.
///
/// Opens a separate `ChainStore` handle to the same database and periodically
/// checks the current chain height (via `Chain`) to prune old blocks.
pub async fn run_light_pruning_loop(chain: Chain, data_dir: &Path) {
    let db_path = data_dir.join("chain.redb");
    let store = match ChainStore::open(&db_path) {
        Ok(s) => s,
        Err(e) => {
            tracing::error!(error = %e, "Light mode: failed to open store for pruning");
            return;
        }
    };

    tracing::info!(
        keep_blocks = LIGHT_MODE_KEEP_BLOCKS,
        "Light mode: pruning loop started"
    );

    let mut interval =
        tokio::time::interval(tokio::time::Duration::from_secs(PRUNE_CHECK_INTERVAL_SECS));

    loop {
        interval.tick().await;
        let current_height = chain.get_block_number();
        prune_old_blocks(&store, current_height, LIGHT_MODE_KEEP_BLOCKS);
    }
}

/// For Fast Sync: request a state snapshot from peers before starting the
/// normal block sync loop. This is called once at startup.
///
/// The actual snapshot download happens via P2P `GetStateSnapshot` requests.
/// This function logs the intent; the real handling occurs in `chain.rs`
/// `handle_sync_response` when a `StateSnapshot` response arrives.
pub fn log_fast_sync_intent() {
    tracing::info!(
        "Fast sync mode: will request state snapshot from peers on first connection"
    );
}

/// For Fast Sync mode: verify that a received state snapshot's state_root matches
/// the hash of the state data.
///
/// Returns `true` if the snapshot is valid, `false` otherwise.
pub fn verify_state_snapshot(state_root: &[u8; 32], state_data: &[u8]) -> bool {
    use sha2::{Digest, Sha256};
    let computed = Sha256::digest(state_data);
    let computed_bytes: [u8; 32] = computed.into();
    computed_bytes == *state_root
}

/// Build a state snapshot response from the current chain store and latest block info.
///
/// Returns `None` if no state snapshot is available.
pub fn build_state_snapshot_response(
    store: &ChainStore,
    latest_height: u64,
    latest_state_root: [u8; 32],
    latest_block: Block,
) -> Option<claw_p2p::SyncResponse> {
    match store.get_state_snapshot() {
        Ok(Some(state_data)) => {
            tracing::debug!(
                height = latest_height,
                state_data_size = state_data.len(),
                "Serving state snapshot to peer"
            );
            Some(claw_p2p::SyncResponse::StateSnapshot {
                height: latest_height,
                state_root: latest_state_root,
                state_data,
                latest_block,
            })
        }
        Ok(None) => {
            tracing::warn!("State snapshot requested but none available");
            None
        }
        Err(e) => {
            tracing::error!(error = %e, "Failed to read state snapshot from storage");
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sync_mode_parse() {
        assert_eq!(SyncMode::parse("full"), SyncMode::Full);
        assert_eq!(SyncMode::parse("fast"), SyncMode::Fast);
        assert_eq!(SyncMode::parse("light"), SyncMode::Light);
        assert_eq!(SyncMode::parse("FAST"), SyncMode::Fast);
        assert_eq!(SyncMode::parse("Light"), SyncMode::Light);
        assert_eq!(SyncMode::parse("unknown"), SyncMode::Full);
        assert_eq!(SyncMode::parse(""), SyncMode::Full);
    }

    #[test]
    fn test_sync_mode_display() {
        assert_eq!(SyncMode::Full.to_string(), "full");
        assert_eq!(SyncMode::Fast.to_string(), "fast");
        assert_eq!(SyncMode::Light.to_string(), "light");
    }

    #[test]
    fn test_verify_state_snapshot() {
        use sha2::{Digest, Sha256};
        let data = b"test state data";
        let hash: [u8; 32] = Sha256::digest(data).into();
        assert!(verify_state_snapshot(&hash, data));
        assert!(!verify_state_snapshot(&[0u8; 32], data));
    }
}
