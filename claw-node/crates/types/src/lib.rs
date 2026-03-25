//! Core data types for ClawNetwork blockchain.
//!
//! Defines all on-chain structures: transactions, blocks, agent identity,
//! tokens, reputation, and service registry.

pub mod block;
pub mod receipt;
pub mod transaction;
pub mod state;

pub use block::{Block, BlockEvent};
pub use receipt::{ReceiptEvent, TransactionReceipt};
pub use transaction::{Transaction, TxType};

#[cfg(test)]
mod tests;
