//! World state machine for ClawNetwork.
//!
//! Applies transactions to the world state, validates rules,
//! and computes state roots.

mod world;
mod handlers;
mod error;
pub mod rewards;

pub use world::WorldState;
pub use error::StateError;

#[cfg(test)]
mod tests;
