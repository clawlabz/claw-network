use borsh::{BorshDeserialize, BorshSerialize};
use serde::{Deserialize, Serialize};

/// Pending upgrade state stored on a contract instance after an announce.
#[derive(Debug, Clone, PartialEq, Eq, BorshSerialize, BorshDeserialize, Serialize, Deserialize)]
pub struct PendingUpgrade {
    /// blake3 hash of the new Wasm code committed in the announce transaction.
    pub new_code_hash: [u8; 32],
    /// Block height at which the upgrade was announced.
    pub announced_at: u64,
    /// Block height at or after which the execute is allowed (`announced_at + UPGRADE_DELAY_BLOCKS`).
    pub ready_at: u64,
}

/// Metadata for a deployed contract.
#[derive(Debug, Clone, PartialEq, Eq, BorshSerialize, BorshDeserialize, Serialize, Deserialize)]
pub struct ContractInstance {
    pub address: [u8; 32],
    pub code_hash: [u8; 32],
    pub creator: [u8; 32],
    pub deployed_at: u64,
    /// Admin address: the only account permitted to announce/execute upgrades.
    /// Defaults to the creator on deploy.
    pub admin: [u8; 32],
    /// Code hash of the previous version, set after a successful upgrade.
    pub previous_code_hash: Option<[u8; 32]>,
    /// Block height at which the most recent upgrade was executed.
    pub upgrade_height: Option<u64>,
    /// Pending upgrade announced but not yet executed.
    pub pending_upgrade: Option<PendingUpgrade>,
}

/// A structured event emitted by a contract during execution.
#[derive(Debug, Clone)]
pub struct ContractEvent {
    /// Event name / category (non-empty UTF-8 string, max 256 bytes).
    pub topic: String,
    /// Arbitrary event payload (max 4096 bytes).
    pub data: Vec<u8>,
}

/// Result of a contract execution.
#[derive(Debug, Clone, Default)]
pub struct ExecutionResult {
    pub return_data: Vec<u8>,
    pub fuel_consumed: u64,
    pub storage_changes: Vec<(Vec<u8>, Option<Vec<u8>>)>,
    pub logs: Vec<String>,
    pub transfers: Vec<([u8; 32], u128)>,
    /// Events emitted by the contract via `emit_event`.
    pub events: Vec<ContractEvent>,
}

/// Context passed to contract execution.
#[derive(Debug, Clone)]
pub struct ExecutionContext {
    pub caller: [u8; 32],
    pub contract_address: [u8; 32],
    pub block_height: u64,
    pub block_timestamp: u64,
    pub value: u128,
    pub fuel_limit: u64,
    /// When `true`, any host function that mutates state (storage write/delete,
    /// token transfer) will trap immediately.  Used for view calls.
    pub read_only: bool,
}

/// Read-only chain state interface for the VM.
pub trait ChainState: Send + Sync {
    fn get_balance(&self, address: &[u8; 32]) -> u128;
    fn get_agent_score(&self, address: &[u8; 32]) -> u64;
    fn get_agent_registered(&self, address: &[u8; 32]) -> bool;
    fn get_contract_storage(&self, contract: &[u8; 32], key: &[u8]) -> Option<Vec<u8>>;
}
