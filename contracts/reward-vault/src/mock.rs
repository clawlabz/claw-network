//! In-process mock environment for unit/integration testing.
//!
//! Replaces the ClawNetwork VM host functions with a simple in-memory store so
//! tests can run with `cargo test` on the native target without any VM or
//! blockchain infrastructure.
//!
//! The mock is intentionally minimal: it holds only the state that the
//! Reward Vault contract cares about.

use std::collections::HashMap;

use crate::types::{claimed_key, nonce_key, platform_key, KEY_OWNER};

/// All-zeros address constant (useful in tests).
pub const ZERO_ADDR: [u8; 32] = [0u8; 32];

// ---------------------------------------------------------------------------
// MockEnv
// ---------------------------------------------------------------------------

/// Simulated execution environment for one transaction.
pub struct MockEnv {
    /// Key → value storage (mirrors contract storage).
    storage: HashMap<Vec<u8>, Vec<u8>>,
    /// Current transaction caller.
    caller: [u8; 32],
    /// Value attached to the current call.
    value: u128,
    /// Current block timestamp (unix seconds).
    timestamp: u64,
    /// Simulated balances: address → nano-CLAW.
    balances: HashMap<[u8; 32], u128>,
    /// Contract's own address (fixed for test convenience).
    pub contract_address: [u8; 32],
}

impl MockEnv {
    /// Create a fresh environment.
    pub fn new() -> Self {
        let contract_address = [0xCCu8; 32];
        let mut balances = HashMap::new();
        balances.insert(contract_address, 0u128);
        MockEnv {
            storage: HashMap::new(),
            caller: ZERO_ADDR,
            value: 0,
            timestamp: 0,
            balances,
            contract_address,
        }
    }

    // -----------------------------------------------------------------------
    // Setters (used by tests to configure environment before each call)
    // -----------------------------------------------------------------------

    pub fn set_caller(&mut self, caller: [u8; 32]) {
        self.caller = caller;
    }

    pub fn set_value(&mut self, value: u128) {
        self.value = value;
    }

    pub fn set_timestamp(&mut self, ts: u64) {
        self.timestamp = ts;
    }

    pub fn set_contract_balance(&mut self, amount: u128) {
        self.balances.insert(self.contract_address, amount);
    }

    // -----------------------------------------------------------------------
    // Host function equivalents (called by logic.rs)
    // -----------------------------------------------------------------------

    pub fn get_caller(&self) -> [u8; 32] {
        self.caller
    }

    pub fn get_timestamp(&self) -> u64 {
        self.timestamp
    }

    pub fn get_contract_address(&self) -> [u8; 32] {
        self.contract_address
    }

    pub fn get_value(&self) -> u128 {
        self.value
    }

    pub fn get_balance(&self, addr: &[u8; 32]) -> u128 {
        self.balances.get(addr).copied().unwrap_or(0)
    }

    /// Transfer `amount` from the contract to `to`.
    ///
    /// Returns `Ok(())` on success, `Err` if the contract has insufficient
    /// funds.
    pub fn transfer(&mut self, to: &[u8; 32], amount: u128) -> Result<(), String> {
        let contract_bal = self
            .balances
            .get(&self.contract_address)
            .copied()
            .unwrap_or(0);
        if contract_bal < amount {
            return Err(format!("mock: insufficient balance ({} < {})", contract_bal, amount));
        }
        *self.balances.entry(self.contract_address).or_insert(0) -= amount;
        *self.balances.entry(*to).or_insert(0) += amount;
        Ok(())
    }

    /// Credit the contract balance (simulates a funded call).
    pub fn credit_contract(&mut self, amount: u128) {
        *self.balances.entry(self.contract_address).or_insert(0) += amount;
    }

    // -----------------------------------------------------------------------
    // Storage primitives
    // -----------------------------------------------------------------------

    pub fn storage_set(&mut self, key: &[u8], value: &[u8]) {
        self.storage.insert(key.to_vec(), value.to_vec());
    }

    pub fn storage_get(&self, key: &[u8]) -> Option<Vec<u8>> {
        self.storage.get(key).cloned()
    }

    pub fn storage_exists(&self, key: &[u8]) -> bool {
        self.storage.contains_key(key)
    }

    pub fn storage_remove(&mut self, key: &[u8]) {
        self.storage.remove(key);
    }

    pub fn panic_msg(msg: &str) -> ! {
        panic!("{}", msg);
    }

    // -----------------------------------------------------------------------
    // Typed helpers for assertions in tests
    // -----------------------------------------------------------------------

    pub fn read_version(&self) -> u32 {
        let bytes = self.storage_get(b"version").expect("version not set");
        borsh::from_slice::<u32>(&bytes).unwrap()
    }

    pub fn read_owner(&self) -> [u8; 32] {
        let bytes = self.storage_get(KEY_OWNER).expect("owner not set");
        borsh::from_slice::<[u8; 32]>(&bytes).unwrap()
    }

    pub fn read_daily_cap(&self) -> u128 {
        let bytes = self.storage_get(b"daily_cap").expect("daily_cap not set");
        u128::from_le_bytes(bytes[..16].try_into().unwrap())
    }

    pub fn read_min_games(&self) -> u64 {
        let bytes = self.storage_get(b"min_games").expect("min_games not set");
        u64::from_le_bytes(bytes[..8].try_into().unwrap())
    }

    pub fn is_paused(&self) -> bool {
        self.storage_get(b"paused")
            .map(|b| b.first().copied().unwrap_or(0))
            .unwrap_or(0)
            == 1
    }

    pub fn is_platform_authorized(&self, addr: &[u8; 32]) -> bool {
        self.storage_get(&platform_key(addr))
            .map(|b| b.first().copied().unwrap_or(0))
            .unwrap_or(0)
            == 1
    }

    pub fn read_nonce(&self, addr: &[u8; 32]) -> u64 {
        let bytes = match self.storage_get(&nonce_key(addr)) {
            Some(b) => b,
            None => return 0,
        };
        u64::from_le_bytes(bytes[..8].try_into().unwrap())
    }

    pub fn contract_balance(&self) -> u128 {
        self.balances.get(&self.contract_address).copied().unwrap_or(0)
    }

    pub fn balance_of(&self, addr: &[u8; 32]) -> u128 {
        self.balances.get(addr).copied().unwrap_or(0)
    }

    pub fn has_claimed_key(&self, addr: &[u8; 32], day: u64) -> bool {
        self.storage_exists(&claimed_key(addr, day))
    }
}

impl Default for MockEnv {
    fn default() -> Self {
        Self::new()
    }
}
