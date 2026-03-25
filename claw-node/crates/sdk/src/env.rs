//! Safe wrappers around the host functions provided by the ClawNetwork VM.
//!
//! All raw `extern "C"` declarations are private. The public API exposes safe
//! Rust functions that handle pointer/length marshalling internally.

/// Maximum buffer size for storage reads (16 KB).
const STORAGE_READ_BUF: usize = 16 * 1024;

// ---------------------------------------------------------------------------
// Raw host function imports (private)
// ---------------------------------------------------------------------------
#[link(wasm_import_module = "env")]
extern "C" {
    fn storage_read(key_ptr: u32, key_len: u32, val_ptr: u32) -> i32;
    fn storage_write(key_ptr: u32, key_len: u32, val_ptr: u32, val_len: u32);
    fn storage_has(key_ptr: u32, key_len: u32) -> i32;
    fn storage_delete(key_ptr: u32, key_len: u32);
    fn caller(out_ptr: u32);
    fn block_height() -> i64;
    fn block_timestamp() -> i64;
    fn contract_address(out_ptr: u32);
    fn value_lo() -> i64;
    fn value_hi() -> i64;
    fn agent_get_score(addr_ptr: u32) -> i64;
    fn agent_is_registered(addr_ptr: u32) -> i32;
    fn token_balance(addr_ptr: u32) -> i64;
    fn token_balance_hi(addr_ptr: u32) -> i64;
    fn token_transfer(to_ptr: u32, amount_lo: i64, amount_hi: i64) -> i32;
    fn log_msg(ptr: u32, len: u32);
    fn return_data(ptr: u32, len: u32);
    fn abort(ptr: u32, len: u32);
    fn emit_event(topic_ptr: u32, topic_len: u32, data_ptr: u32, data_len: u32);
    fn call_contract(
        addr_ptr: u32,
        method_ptr: u32, method_len: u32,
        args_ptr: u32, args_len: u32,
        value_lo: i64, value_hi: i64,
    ) -> i32;
    fn cross_call_return_data(out_ptr: u32) -> i32;
}

// ---------------------------------------------------------------------------
// Public safe API
// ---------------------------------------------------------------------------

/// Get the transaction caller's address.
pub fn get_caller() -> [u8; 32] {
    let mut buf = [0u8; 32];
    unsafe { caller(buf.as_mut_ptr() as u32) };
    buf
}

/// Get current block height.
pub fn get_block_height() -> u64 {
    unsafe { block_height() as u64 }
}

/// Get current block timestamp (unix seconds).
pub fn get_block_timestamp() -> u64 {
    unsafe { block_timestamp() as u64 }
}

/// Get this contract's address.
pub fn get_contract_address() -> [u8; 32] {
    let mut buf = [0u8; 32];
    unsafe { contract_address(buf.as_mut_ptr() as u32) };
    buf
}

/// Get the CLAW value transferred with this call.
pub fn get_value() -> u128 {
    let lo = unsafe { value_lo() } as u64;
    let hi = unsafe { value_hi() } as u64;
    (hi as u128) << 64 | (lo as u128)
}

/// Read a value from contract storage.
///
/// Returns `None` if the key does not exist or the value exceeds the 16 KB
/// internal buffer.
pub fn storage_get(key: &[u8]) -> Option<Vec<u8>> {
    let mut buf = vec![0u8; STORAGE_READ_BUF];
    let len = unsafe {
        storage_read(
            key.as_ptr() as u32,
            key.len() as u32,
            buf.as_mut_ptr() as u32,
        )
    };
    if len < 0 {
        None
    } else {
        buf.truncate(len as usize);
        Some(buf)
    }
}

/// Write a value to contract storage.
pub fn storage_set(key: &[u8], value: &[u8]) {
    unsafe {
        storage_write(
            key.as_ptr() as u32,
            key.len() as u32,
            value.as_ptr() as u32,
            value.len() as u32,
        );
    }
}

/// Check if a key exists in storage.
pub fn storage_exists(key: &[u8]) -> bool {
    unsafe { storage_has(key.as_ptr() as u32, key.len() as u32) != 0 }
}

/// Delete a key from storage.
pub fn storage_remove(key: &[u8]) {
    unsafe { storage_delete(key.as_ptr() as u32, key.len() as u32) }
}

/// Get an agent's reputation score (0–100).
pub fn get_agent_score(address: &[u8; 32]) -> u64 {
    unsafe { agent_get_score(address.as_ptr() as u32) as u64 }
}

/// Check if an address is a registered agent.
pub fn is_agent_registered(address: &[u8; 32]) -> bool {
    unsafe { agent_is_registered(address.as_ptr() as u32) != 0 }
}

/// Get CLAW balance of an address as the full u128.
///
/// Combines the `token_balance` (low 64 bits) and `token_balance_hi`
/// (high 64 bits) host functions, matching the lo/hi pattern used by
/// `get_value()` for transferred amounts.
pub fn get_balance(address: &[u8; 32]) -> u128 {
    let lo = unsafe { token_balance(address.as_ptr() as u32) } as u64;
    let hi = unsafe { token_balance_hi(address.as_ptr() as u32) } as u64;
    (hi as u128) << 64 | (lo as u128)
}

/// Transfer CLAW tokens from the contract to an address.
///
/// Returns `true` on success, `false` on failure (e.g. insufficient balance).
pub fn transfer(to: &[u8; 32], amount: u128) -> bool {
    let lo = amount as i64;
    let hi = (amount >> 64) as i64;
    unsafe { token_transfer(to.as_ptr() as u32, lo, hi) != 0 }
}

/// Emit a log message.
pub fn log(msg: &str) {
    unsafe { log_msg(msg.as_ptr() as u32, msg.len() as u32) }
}

/// Set the return data for this execution.
pub fn set_return_data(data: &[u8]) {
    unsafe { return_data(data.as_ptr() as u32, data.len() as u32) }
}

/// Emit a structured contract event.
///
/// The `topic` must be a non-empty UTF-8 string (max 256 bytes).
/// The `data` is an arbitrary byte payload (max 4096 bytes).
///
/// Traps if:
/// - `topic` is empty or exceeds 256 bytes
/// - `data` exceeds 4096 bytes
/// - the per-execution event limit (50) has been reached
pub fn emit_event_raw(topic: &str, data: &[u8]) {
    unsafe {
        emit_event(
            topic.as_ptr() as u32,
            topic.len() as u32,
            data.as_ptr() as u32,
            data.len() as u32,
        )
    }
}

/// Maximum buffer size for cross-contract call return data (16 KB).
const CROSS_CALL_RETURN_BUF: usize = 16 * 1024;

/// Call another deployed contract synchronously.
///
/// Returns `Ok(return_data)` on success, or an `Err` variant on failure:
/// - `Err(CrossCallError::Failed)` — child contract reverted or not found
/// - `Err(CrossCallError::Reentrancy)` — reentrancy detected (A→B→A)
/// - `Err(CrossCallError::MaxDepth)` — call depth exceeded (max 4)
pub fn call_contract_invoke(
    address: &[u8; 32],
    method: &str,
    args: &[u8],
    value: u128,
) -> Result<Vec<u8>, CrossCallError> {
    let lo = value as i64;
    let hi = (value >> 64) as i64;
    let result = unsafe {
        call_contract(
            address.as_ptr() as u32,
            method.as_ptr() as u32,
            method.len() as u32,
            args.as_ptr() as u32,
            args.len() as u32,
            lo,
            hi,
        )
    };
    match result {
        0 => {
            // Success — read return data
            let mut buf = vec![0u8; CROSS_CALL_RETURN_BUF];
            let len = unsafe { cross_call_return_data(buf.as_mut_ptr() as u32) };
            if len < 0 {
                return Ok(Vec::new());
            }
            buf.truncate(len as usize);
            Ok(buf)
        }
        -2 => Err(CrossCallError::Reentrancy),
        -3 => Err(CrossCallError::MaxDepth),
        _ => Err(CrossCallError::Failed),
    }
}

/// Errors from cross-contract calls.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CrossCallError {
    /// Child contract reverted or was not found.
    Failed,
    /// Reentrancy detected (contract A called B which tried to call A).
    Reentrancy,
    /// Maximum call depth exceeded (4 levels).
    MaxDepth,
}

/// Abort execution with an error message.
///
/// This function never returns.
pub fn panic_msg(msg: &str) -> ! {
    unsafe { abort(msg.as_ptr() as u32, msg.len() as u32) };
    // The host `abort` terminates execution, but the compiler doesn't know that.
    // This unreachable loop satisfies the `-> !` return type.
    #[allow(clippy::empty_loop)]
    loop {}
}
