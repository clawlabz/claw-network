use std::collections::HashSet;
use std::sync::Arc;

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
    /// Current nesting depth for cross-contract calls (0 = top-level).
    pub call_depth: u32,
    /// MANDATORY reentrancy mutex: set of contract addresses currently on the
    /// call stack.  Any reentrant call (A→B→A) is rejected immediately.
    pub executing_contracts: Arc<std::sync::Mutex<HashSet<[u8; 32]>>>,
}

impl ExecutionContext {
    /// Create a new top-level execution context (call_depth = 0, empty mutex).
    pub fn new_top_level(
        caller: [u8; 32],
        contract_address: [u8; 32],
        block_height: u64,
        block_timestamp: u64,
        value: u128,
        fuel_limit: u64,
        read_only: bool,
    ) -> Self {
        let executing = Arc::new(std::sync::Mutex::new(HashSet::new()));
        executing.lock().unwrap().insert(contract_address);
        Self {
            caller,
            contract_address,
            block_height,
            block_timestamp,
            value,
            fuel_limit,
            read_only,
            call_depth: 0,
            executing_contracts: executing,
        }
    }
}

/// Read-only chain state interface for the VM.
pub trait ChainState: Send + Sync {
    fn get_balance(&self, address: &[u8; 32]) -> u128;
    fn get_agent_score(&self, address: &[u8; 32]) -> u64;
    fn get_agent_registered(&self, address: &[u8; 32]) -> bool;
    fn get_contract_storage(&self, contract: &[u8; 32], key: &[u8]) -> Option<Vec<u8>>;
    /// Retrieve compiled Wasm bytecode for a deployed contract.
    /// Required for cross-contract calls.
    fn get_contract_code(&self, contract: &[u8; 32]) -> Option<Vec<u8>>;
}

/// Result of a cross-contract call, used to merge child state into parent.
#[derive(Debug, Clone, Default)]
pub struct CrossCallResult {
    pub success: bool,
    pub return_data: Vec<u8>,
    pub fuel_consumed: u64,
    /// Storage changes keyed by (contract_address, key, value).
    /// `None` value means deletion.
    pub storage_changes: Vec<([u8; 32], Vec<u8>, Option<Vec<u8>>)>,
    /// Token transfers: (recipient, amount).
    pub transfers: Vec<([u8; 32], u128)>,
    /// Events emitted by the child contract.
    pub events: Vec<ContractEvent>,
}
