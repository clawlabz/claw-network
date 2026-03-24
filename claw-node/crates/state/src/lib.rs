//! World state machine for ClawNetwork.
//!
//! Applies transactions to the world state, validates rules,
//! and computes state roots.

mod world;
mod handlers;
mod error;
pub mod rewards;
pub mod score;

pub use world::WorldState;
pub use error::StateError;
pub use score::{compute_agent_score, AgentScoreDetail};

/// Number of blocks that must elapse between a `ContractUpgradeAnnounce` and
/// a `ContractUpgradeExecute`. At ~3 s/block this is approximately 36 minutes,
/// giving node operators and users time to react before an upgrade takes effect.
pub const UPGRADE_DELAY_BLOCKS: u64 = 720;

#[cfg(test)]
mod tests;
