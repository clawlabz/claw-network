//! Transaction receipt types for post-execution results.

use borsh::{BorshDeserialize, BorshSerialize};
use serde::{Deserialize, Serialize};

/// An event emitted during contract execution, stored in the receipt.
///
/// This is a standalone type (not dependent on claw_vm) so it can live
/// in claw_types without circular dependencies.
#[derive(Debug, Clone, BorshSerialize, BorshDeserialize, Serialize, Deserialize)]
pub struct ReceiptEvent {
    /// Event name / category.
    pub topic: String,
    /// Arbitrary event payload.
    pub data: Vec<u8>,
}

/// The result of executing a transaction, persisted per-block.
#[derive(Debug, Clone, BorshSerialize, BorshDeserialize, Serialize, Deserialize)]
pub struct TransactionReceipt {
    /// Hash of the transaction this receipt belongs to.
    pub tx_hash: [u8; 32],
    /// Whether the transaction executed successfully.
    pub success: bool,
    /// Fuel (compute units) consumed during execution.
    pub fuel_consumed: u64,
    /// Fuel limit that was set for this execution.
    pub fuel_limit: u64,
    /// Data returned by the contract (empty for non-contract txs).
    pub return_data: Vec<u8>,
    /// Error message if the transaction failed.
    pub error_message: Option<String>,
    /// Events emitted during contract execution.
    pub events: Vec<ReceiptEvent>,
    /// Log messages from contract execution.
    pub logs: Vec<String>,
}
