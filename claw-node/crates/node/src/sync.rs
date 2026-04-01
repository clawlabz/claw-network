//! Sync mode logic: Full, Fast, and Light node support.
//!
//! - **Full**: Download and store all blocks (default behavior).
//! - **Fast**: Download latest state snapshot from a peer, verify state_root,
//!   then sync only recent blocks.
//! - **Light**: Prune blocks older than N, keep only state + recent blocks.

use std::path::Path;

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

/// Run a periodic pruning loop for Light mode.
///
/// Reuses the Chain's already-open ChainStore handle to avoid redb exclusive
/// lock errors. The old approach of opening a second ChainStore handle caused
/// "Database already open. Cannot acquire lock." failures.
pub async fn run_light_pruning_loop(chain: Chain, _data_dir: &Path) {
    tracing::info!(
        keep_blocks = LIGHT_MODE_KEEP_BLOCKS,
        "Light mode: pruning loop started"
    );

    let mut interval =
        tokio::time::interval(tokio::time::Duration::from_secs(PRUNE_CHECK_INTERVAL_SECS));

    loop {
        interval.tick().await;
        chain.prune_old_blocks(LIGHT_MODE_KEEP_BLOCKS);
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
}
