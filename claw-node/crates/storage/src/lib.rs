//! Persistent storage layer for ClawNetwork using redb.

mod store;

pub use store::ChainStore;

#[cfg(test)]
mod tests;
