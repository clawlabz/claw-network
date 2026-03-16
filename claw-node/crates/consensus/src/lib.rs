//! PoS + Agent Score consensus engine for ClawNetwork.
//!
//! Handles validator selection, block proposal, and BFT voting.

mod types;
mod validator_set;
mod election;
mod voting;
pub mod slashing;

#[cfg(test)]
mod tests;

pub use types::*;
pub use validator_set::ValidatorSet;
pub use election::{elect_proposer, elect_fallback_proposer};
pub use voting::{VoteCollector, VoteResult};
pub use slashing::{SlashingState, EquivocationEvidence};
