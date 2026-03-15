//! Safe wrappers around the 17 host functions provided by the ClawNetwork VM.
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
    fn token_transfer(to_ptr: u32, amount_lo: i64, amount_hi: i64) -> i32;
    fn log_msg(ptr: u32, len: u32);
    fn return_data(ptr: u32, len: u32);
    fn abort(ptr: u32, len: u32);
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

/// Get CLAW balance of an address.
pub fn get_balance(address: &[u8; 32]) -> u64 {
    unsafe { token_balance(address.as_ptr() as u32) as u64 }
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
