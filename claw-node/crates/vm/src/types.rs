use borsh::{BorshDeserialize, BorshSerialize};
use serde::{Deserialize, Serialize};

/// Metadata for a deployed contract.
#[derive(Debug, Clone, PartialEq, Eq, BorshSerialize, BorshDeserialize, Serialize, Deserialize)]
pub struct ContractInstance {
    pub address: [u8; 32],
    pub code_hash: [u8; 32],
    pub creator: [u8; 32],
    pub deployed_at: u64,
}

/// Result of a contract execution.
#[derive(Debug, Clone, Default)]
pub struct ExecutionResult {
    pub return_data: Vec<u8>,
    pub fuel_consumed: u64,
    pub storage_changes: Vec<(Vec<u8>, Option<Vec<u8>>)>,
    pub logs: Vec<String>,
    pub transfers: Vec<([u8; 32], u128)>,
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
}

/// Read-only chain state interface for the VM.
pub trait ChainState: Send + Sync {
    fn get_balance(&self, address: &[u8; 32]) -> u128;
    fn get_agent_score(&self, address: &[u8; 32]) -> u64;
    fn get_agent_registered(&self, address: &[u8; 32]) -> bool;
    fn get_contract_storage(&self, contract: &[u8; 32], key: &[u8]) -> Option<Vec<u8>>;
}
