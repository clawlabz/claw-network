use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};

use wasmer::{AsStoreRef, FunctionEnvMut, Memory};

use crate::constants::*;
use crate::error::VmError;
use crate::types::ExecutionContext;

/// Maximum buffer size for guest-provided lengths in host functions (64 KB).
/// Prevents a malicious contract from requesting an unbounded allocation.
const MAX_HOST_BUFFER_SIZE: usize = 65536;

/// Shared environment accessible by host functions.
#[derive(Clone)]
pub struct HostEnv {
    pub context: ExecutionContext,
    /// Contract storage: initial snapshot merged with pending writes.
    pub storage: Arc<Mutex<BTreeMap<Vec<u8>, Vec<u8>>>>,
    /// Pending storage changes for the execution result.
    pub storage_changes: Arc<Mutex<Vec<(Vec<u8>, Option<Vec<u8>>)>>>,
    /// Log messages.
    pub logs: Arc<Mutex<Vec<String>>>,
    /// Token transfers.
    pub transfers: Arc<Mutex<Vec<([u8; 32], u128)>>>,
    /// Return data set by the contract.
    pub return_data: Arc<Mutex<Vec<u8>>>,
    /// Chain state snapshots for read-only queries.
    pub balances: Arc<BTreeMap<[u8; 32], u128>>,
    pub agent_scores: Arc<BTreeMap<[u8; 32], u64>>,
    pub registered_agents: Arc<BTreeSet<[u8; 32]>>,
    /// Reference to Wasm memory (set after instantiation).
    pub memory: Option<Memory>,
    /// Manual fuel tracking: remaining fuel.
    pub fuel_remaining: Arc<Mutex<u64>>,
    /// Manual fuel tracking: total consumed.
    pub fuel_consumed: Arc<Mutex<u64>>,
    /// Fuel limit for error reporting.
    pub fuel_limit: u64,
}

impl HostEnv {
    /// Deduct fuel, returning an error string if exhausted.
    fn consume_fuel(
        fuel_remaining: &Mutex<u64>,
        fuel_consumed: &Mutex<u64>,
        cost: u64,
        fuel_limit: u64,
    ) -> Result<(), String> {
        let mut remaining = fuel_remaining.lock().unwrap();
        let mut consumed = fuel_consumed.lock().unwrap();
        if *remaining < cost {
            let used = *consumed + (*remaining);
            Err(format!(
                "fuel exhausted: used {used}, limit {fuel_limit}"
            ))
        } else {
            *remaining -= cost;
            *consumed += cost;
            Ok(())
        }
    }
}

// ---------------------------------------------------------------------------
// Memory helpers
// ---------------------------------------------------------------------------

fn read_bytes(env: &FunctionEnvMut<HostEnv>, ptr: u32, len: u32) -> Result<Vec<u8>, VmError> {
    if len as usize > MAX_HOST_BUFFER_SIZE {
        return Err(VmError::MemoryError(format!(
            "guest requested {} bytes, max is {}",
            len, MAX_HOST_BUFFER_SIZE
        )));
    }
    let env_data = env.data();
    let memory = env_data
        .memory
        .as_ref()
        .ok_or_else(|| VmError::MemoryError("memory not set".to_string()))?;
    let store_ref = env.as_store_ref();
    let mem_view = memory.view(&store_ref);
    let mut buf = vec![0u8; len as usize];
    mem_view
        .read(ptr as u64, &mut buf)
        .map_err(|e| VmError::MemoryError(e.to_string()))?;
    Ok(buf)
}

fn write_bytes(env: &FunctionEnvMut<HostEnv>, ptr: u32, data: &[u8]) -> Result<(), VmError> {
    let env_data = env.data();
    let memory = env_data
        .memory
        .as_ref()
        .ok_or_else(|| VmError::MemoryError("memory not set".to_string()))?;
    let store_ref = env.as_store_ref();
    let mem_view = memory.view(&store_ref);
    mem_view
        .write(ptr as u64, data)
        .map_err(|e| VmError::MemoryError(e.to_string()))?;
    Ok(())
}

/// Deducts fuel from the execution budget.
/// # Panics
/// Intentionally panics on fuel exhaustion — Wasmer's catch_unwind converts this to a trap.
fn consume_fuel_or_trap(env: &FunctionEnvMut<HostEnv>, cost: u64) {
    let data = env.data();
    if let Err(msg) = HostEnv::consume_fuel(
        &data.fuel_remaining,
        &data.fuel_consumed,
        cost,
        data.fuel_limit,
    ) {
        panic!("out of fuel: {msg}");
    }
}

// ---------------------------------------------------------------------------
// Storage host functions
// ---------------------------------------------------------------------------

/// Read a value from contract storage.
/// Returns the byte length of the value, or -1 if the key does not exist.
/// The value is written to `val_ptr` in guest memory.
pub fn host_storage_read(
    env: FunctionEnvMut<HostEnv>,
    key_ptr: u32,
    key_len: u32,
    val_ptr: u32,
) -> i32 {
    consume_fuel_or_trap(&env, STORAGE_READ_FUEL);
    let key = match read_bytes(&env, key_ptr, key_len) {
        Ok(k) => k,
        Err(_) => return -1,
    };
    let storage = env.data().storage.lock().unwrap();
    match storage.get(&key) {
        Some(val) => {
            let len = val.len() as i32;
            let val_clone = val.clone();
            drop(storage);
            if let Err(_) = write_bytes(&env, val_ptr, &val_clone) {
                return -1;
            }
            len
        }
        None => -1,
    }
}

/// Write a value to contract storage.
pub fn host_storage_write(
    env: FunctionEnvMut<HostEnv>,
    key_ptr: u32,
    key_len: u32,
    val_ptr: u32,
    val_len: u32,
) {
    consume_fuel_or_trap(&env, STORAGE_WRITE_FUEL);
    let key = match read_bytes(&env, key_ptr, key_len) {
        Ok(k) => k,
        Err(_) => return,
    };
    let val = match read_bytes(&env, val_ptr, val_len) {
        Ok(v) => v,
        Err(_) => return,
    };
    let data = env.data();
    data.storage.lock().unwrap().insert(key.clone(), val.clone());
    data.storage_changes
        .lock()
        .unwrap()
        .push((key, Some(val)));
}

/// Check whether a key exists in contract storage. Returns 1 if yes, 0 if no.
pub fn host_storage_has(env: FunctionEnvMut<HostEnv>, key_ptr: u32, key_len: u32) -> i32 {
    consume_fuel_or_trap(&env, STORAGE_READ_FUEL);
    let key = match read_bytes(&env, key_ptr, key_len) {
        Ok(k) => k,
        Err(_) => return 0,
    };
    let storage = env.data().storage.lock().unwrap();
    if storage.contains_key(&key) {
        1
    } else {
        0
    }
}

/// Delete a key from contract storage.
pub fn host_storage_delete(env: FunctionEnvMut<HostEnv>, key_ptr: u32, key_len: u32) {
    consume_fuel_or_trap(&env, STORAGE_DELETE_FUEL);
    let key = match read_bytes(&env, key_ptr, key_len) {
        Ok(k) => k,
        Err(_) => return,
    };
    let data = env.data();
    data.storage.lock().unwrap().remove(&key);
    data.storage_changes.lock().unwrap().push((key, None));
}

// ---------------------------------------------------------------------------
// Context host functions
// ---------------------------------------------------------------------------

/// Write the 32-byte caller address to guest memory at `out_ptr`.
pub fn host_caller(env: FunctionEnvMut<HostEnv>, out_ptr: u32) {
    consume_fuel_or_trap(&env, HOST_CALL_BASE_FUEL);
    let caller = env.data().context.caller;
    let _ = write_bytes(&env, out_ptr, &caller);
}

/// Return the current block height.
pub fn host_block_height(env: FunctionEnvMut<HostEnv>) -> i64 {
    consume_fuel_or_trap(&env, HOST_CALL_BASE_FUEL);
    env.data().context.block_height as i64
}

/// Return the current block timestamp.
pub fn host_block_timestamp(env: FunctionEnvMut<HostEnv>) -> i64 {
    consume_fuel_or_trap(&env, HOST_CALL_BASE_FUEL);
    env.data().context.block_timestamp as i64
}

/// Write the 32-byte contract address to guest memory at `out_ptr`.
pub fn host_contract_address(env: FunctionEnvMut<HostEnv>, out_ptr: u32) {
    consume_fuel_or_trap(&env, HOST_CALL_BASE_FUEL);
    let addr = env.data().context.contract_address;
    let _ = write_bytes(&env, out_ptr, &addr);
}

/// Return the low 64 bits of the transferred value.
pub fn host_value_lo(env: FunctionEnvMut<HostEnv>) -> i64 {
    consume_fuel_or_trap(&env, HOST_CALL_BASE_FUEL);
    (env.data().context.value & 0xFFFF_FFFF_FFFF_FFFF) as i64
}

/// Return the high 64 bits of the transferred value.
pub fn host_value_hi(env: FunctionEnvMut<HostEnv>) -> i64 {
    consume_fuel_or_trap(&env, HOST_CALL_BASE_FUEL);
    (env.data().context.value >> 64) as i64
}

// ---------------------------------------------------------------------------
// Agent host functions
// ---------------------------------------------------------------------------

/// Get the reputation score of an agent by address.
pub fn host_agent_get_score(env: FunctionEnvMut<HostEnv>, addr_ptr: u32) -> i64 {
    consume_fuel_or_trap(&env, AGENT_QUERY_FUEL);
    let addr_bytes = match read_bytes(&env, addr_ptr, 32) {
        Ok(b) => b,
        Err(_) => return 0,
    };
    let mut addr = [0u8; 32];
    addr.copy_from_slice(&addr_bytes);
    let scores = &env.data().agent_scores;
    scores.get(&addr).copied().unwrap_or(0) as i64
}

/// Check whether an agent is registered. Returns 1 if yes, 0 if no.
pub fn host_agent_is_registered(env: FunctionEnvMut<HostEnv>, addr_ptr: u32) -> i32 {
    consume_fuel_or_trap(&env, AGENT_QUERY_FUEL);
    let addr_bytes = match read_bytes(&env, addr_ptr, 32) {
        Ok(b) => b,
        Err(_) => return 0,
    };
    let mut addr = [0u8; 32];
    addr.copy_from_slice(&addr_bytes);
    if env.data().registered_agents.contains(&addr) {
        1
    } else {
        0
    }
}

// ---------------------------------------------------------------------------
// Token host functions
// ---------------------------------------------------------------------------

/// Return the low 64 bits of an address's token balance.
pub fn host_token_balance(env: FunctionEnvMut<HostEnv>, addr_ptr: u32) -> i64 {
    consume_fuel_or_trap(&env, HOST_CALL_BASE_FUEL);
    let addr_bytes = match read_bytes(&env, addr_ptr, 32) {
        Ok(b) => b,
        Err(_) => return 0,
    };
    let mut addr = [0u8; 32];
    addr.copy_from_slice(&addr_bytes);
    let balances = &env.data().balances;
    let balance = balances.get(&addr).copied().unwrap_or(0);
    (balance & 0xFFFF_FFFF_FFFF_FFFF) as i64
}

/// Transfer tokens to `to_ptr` address. Returns 0 on success, -1 on failure.
pub fn host_token_transfer(
    env: FunctionEnvMut<HostEnv>,
    to_ptr: u32,
    amount_lo: i64,
    amount_hi: i64,
) -> i32 {
    consume_fuel_or_trap(&env, TOKEN_TRANSFER_FUEL);
    let to_bytes = match read_bytes(&env, to_ptr, 32) {
        Ok(b) => b,
        Err(_) => return -1,
    };
    let mut to = [0u8; 32];
    to.copy_from_slice(&to_bytes);
    let amount = (amount_lo as u64 as u128) | ((amount_hi as u64 as u128) << 64);
    if amount == 0 {
        return -1;
    }
    env.data().transfers.lock().unwrap().push((to, amount));
    0
}

// ---------------------------------------------------------------------------
// Utility host functions
// ---------------------------------------------------------------------------

/// Log a message from the contract.
pub fn host_log(env: FunctionEnvMut<HostEnv>, ptr: u32, len: u32) {
    consume_fuel_or_trap(&env, HOST_CALL_BASE_FUEL);
    let bytes = match read_bytes(&env, ptr, len) {
        Ok(b) => b,
        Err(_) => return,
    };
    let msg = String::from_utf8_lossy(&bytes).to_string();
    tracing::info!(target: "claw_vm", "contract log: {}", msg);
    const MAX_LOG_ENTRIES: usize = 100;
    let mut logs = env.data().logs.lock().unwrap();
    if logs.len() >= MAX_LOG_ENTRIES {
        return; // silently drop excess log entries
    }
    logs.push(msg);
}

/// Set the return data for the execution result.
pub fn host_return_data(env: FunctionEnvMut<HostEnv>, ptr: u32, len: u32) {
    consume_fuel_or_trap(&env, HOST_CALL_BASE_FUEL);
    let bytes = match read_bytes(&env, ptr, len) {
        Ok(b) => b,
        Err(_) => return,
    };
    *env.data().return_data.lock().unwrap() = bytes;
}

/// Return the high 64 bits (bits 64–127) of an address's token balance.
///
/// Contracts that need the full u128 balance call both `token_balance` (lo)
/// and `token_balance_hi` (hi) and combine them:
///   balance = (hi as u128) << 64 | (lo as u128)
pub fn host_token_balance_hi(env: FunctionEnvMut<HostEnv>, addr_ptr: u32) -> i64 {
    consume_fuel_or_trap(&env, HOST_CALL_BASE_FUEL);
    let addr_bytes = match read_bytes(&env, addr_ptr, 32) {
        Ok(b) => b,
        Err(_) => return 0,
    };
    let mut addr = [0u8; 32];
    addr.copy_from_slice(&addr_bytes);
    let balances = &env.data().balances;
    let balance = balances.get(&addr).copied().unwrap_or(0);
    (balance >> 64) as i64
}

/// Abort execution with an error message.
///
/// Panics to trap execution. Wasmer catches the panic from host functions
/// and converts it into a `RuntimeError` returned from `Instance::call()`.
pub fn host_abort(env: FunctionEnvMut<HostEnv>, ptr: u32, len: u32) {
    let bytes = read_bytes(&env, ptr, len).unwrap_or_default();
    let msg = String::from_utf8_lossy(&bytes).to_string();
    panic!("contract abort: {msg}");
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    /// Verify the lo/hi split and reconstruction logic for balances.
    ///
    /// These tests mirror exactly what `host_token_balance` and
    /// `host_token_balance_hi` compute, independent of Wasmer machinery.
    fn balance_lo(balance: u128) -> i64 {
        (balance & 0xFFFF_FFFF_FFFF_FFFF) as i64
    }

    fn balance_hi(balance: u128) -> i64 {
        (balance >> 64) as i64
    }

    fn reconstruct(lo: i64, hi: i64) -> u128 {
        (hi as u64 as u128) << 64 | (lo as u64 as u128)
    }

    #[test]
    fn test_balance_lo_hi_zero() {
        let balance: u128 = 0;
        assert_eq!(balance_lo(balance), 0);
        assert_eq!(balance_hi(balance), 0);
        assert_eq!(reconstruct(balance_lo(balance), balance_hi(balance)), 0);
    }

    #[test]
    fn test_balance_fits_in_u64() {
        // A balance that fits entirely in the low 64 bits — hi must be 0.
        let balance: u128 = u64::MAX as u128;
        assert_eq!(balance_lo(balance), -1_i64); // u64::MAX reinterpreted as i64
        assert_eq!(balance_hi(balance), 0);
        assert_eq!(reconstruct(balance_lo(balance), balance_hi(balance)), balance);
    }

    #[test]
    fn test_balance_just_above_u64_max() {
        // 2^64 — first value where high bits are non-zero.
        let balance: u128 = (u64::MAX as u128) + 1;
        assert_eq!(balance_lo(balance), 0);  // low word is 0
        assert_eq!(balance_hi(balance), 1);  // high word is 1
        assert_eq!(reconstruct(balance_lo(balance), balance_hi(balance)), balance);
    }

    #[test]
    fn test_balance_large_u128() {
        // A balance with both lo and hi parts non-zero.
        // balance = 0xDEAD_BEEF_0000_0000_CAFE_BABE_1234_5678
        let hi_word: u64 = 0xDEAD_BEEF_0000_0000;
        let lo_word: u64 = 0xCAFE_BABE_1234_5678;
        let balance: u128 = (hi_word as u128) << 64 | (lo_word as u128);

        assert_eq!(balance_lo(balance), lo_word as i64);
        assert_eq!(balance_hi(balance), hi_word as i64);
        assert_eq!(reconstruct(balance_lo(balance), balance_hi(balance)), balance);
    }

    #[test]
    fn test_balance_max_u128() {
        let balance = u128::MAX;
        // Both lo and hi are all-ones (i64 -1).
        assert_eq!(balance_lo(balance), -1_i64);
        assert_eq!(balance_hi(balance), -1_i64);
        assert_eq!(reconstruct(balance_lo(balance), balance_hi(balance)), balance);
    }

    #[test]
    fn test_old_api_truncates_large_balance() {
        // This documents the bug that existed before: the old `token_balance`
        // host function returned only lo bits, so callers treating the result
        // as the full balance would get a wrong (truncated) value.
        let balance: u128 = ((u64::MAX as u128) + 1) * 5; // 5 * 2^64
        let old_result = balance_lo(balance) as u64;
        assert_eq!(old_result, 0, "old API silently truncates to 0 for multiples of 2^64");
        // The correct full value requires combining lo + hi:
        let full = reconstruct(balance_lo(balance), balance_hi(balance));
        assert_eq!(full, balance);
    }
}
