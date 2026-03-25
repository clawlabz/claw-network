//! Integration tests for the `token_balance_hi` host function.
//!
//! These tests verify that the full u128 balance can be recovered by
//! combining `token_balance` (lo bits) and `token_balance_hi` (hi bits).
//!
//! TDD flow:
//!   RED   — tests written before implementation
//!   GREEN — implementation added to host.rs / engine.rs / env.rs
//!   CHECK — verify old contracts (lo-only) still work (backward compat)

use std::collections::BTreeMap;

use claw_vm::{ChainState, ExecutionContext, VmEngine};

// ---------------------------------------------------------------------------
// Minimal ChainState stub
// ---------------------------------------------------------------------------

struct TestChainState {
    balances: BTreeMap<[u8; 32], u128>,
}

impl ChainState for TestChainState {
    fn get_balance(&self, address: &[u8; 32]) -> u128 {
        self.balances.get(address).copied().unwrap_or(0)
    }
    fn get_agent_score(&self, _: &[u8; 32]) -> u64 {
        0
    }
    fn get_agent_registered(&self, _: &[u8; 32]) -> bool {
        false
    }
    fn get_contract_storage(&self, _: &[u8; 32], _: &[u8]) -> Option<Vec<u8>> {
        None
    }
    fn get_contract_code(&self, _contract: &[u8; 32]) -> Option<Vec<u8>> {
        None
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn make_context(contract_address: [u8; 32]) -> ExecutionContext {
    ExecutionContext::new_top_level(
        [0u8; 32],
        contract_address,
        1,
        0,
        0,
        claw_vm::DEFAULT_FUEL_LIMIT,
        false,
    )
}

/// Compile WAT source to Wasm bytes using the `wat` crate.
fn compile_wat(src: &str) -> Vec<u8> {
    wat::parse_str(src).expect("WAT compilation failed")
}

// ---------------------------------------------------------------------------
// WAT contracts
// ---------------------------------------------------------------------------

/// A contract that queries the balance of its own address using ONLY the
/// legacy `token_balance` (lo) host function and sets return_data to the
/// 8-byte little-endian lo word.
///
/// This is the old, broken contract pattern — it only sees the low 64 bits.
const CONTRACT_BALANCE_LO_ONLY: &str = r#"
(module
  (import "env" "token_balance"    (func $token_balance    (param i32) (result i64)))
  (import "env" "contract_address" (func $contract_address (param i32)))
  (import "env" "return_data"      (func $return_data      (param i32 i32)))
  (import "env" "storage_read"     (func $storage_read     (param i32 i32 i32) (result i32)))
  (import "env" "storage_write"    (func $storage_write    (param i32 i32 i32 i32)))
  (import "env" "storage_has"      (func $storage_has      (param i32 i32) (result i32)))
  (import "env" "storage_delete"   (func $storage_delete   (param i32 i32)))
  (import "env" "caller"           (func $caller           (param i32)))
  (import "env" "block_height"     (func $block_height     (result i64)))
  (import "env" "block_timestamp"  (func $block_timestamp  (result i64)))
  (import "env" "value_lo"         (func $value_lo         (result i64)))
  (import "env" "value_hi"         (func $value_hi         (result i64)))
  (import "env" "agent_get_score"  (func $agent_get_score  (param i32) (result i64)))
  (import "env" "agent_is_registered" (func $agent_is_registered (param i32) (result i32)))
  (import "env" "token_balance_hi" (func $token_balance_hi (param i32) (result i64)))
  (import "env" "token_transfer"   (func $token_transfer   (param i32 i64 i64) (result i32)))
  (import "env" "log_msg"          (func $log_msg          (param i32 i32)))
  (import "env" "abort"            (func $abort            (param i32 i32)))

  (memory (export "memory") 1)

  ;; offset 0:  32-byte address buffer
  ;; offset 32: 8-byte return buffer

  (func (export "query_balance_lo")
    ;; get own address into mem[0]
    (call $contract_address (i32.const 0))
    ;; call token_balance(addr=0) → lo
    (i64.store (i32.const 32) (call $token_balance (i32.const 0)))
    ;; return_data(ptr=32, len=8)
    (call $return_data (i32.const 32) (i32.const 8))
  )
)
"#;

/// A contract that queries balance using BOTH lo and hi, combining them into
/// a full u128, and sets return_data to the 16-byte little-endian u128.
const CONTRACT_BALANCE_FULL_U128: &str = r#"
(module
  (import "env" "token_balance"    (func $token_balance    (param i32) (result i64)))
  (import "env" "token_balance_hi" (func $token_balance_hi (param i32) (result i64)))
  (import "env" "contract_address" (func $contract_address (param i32)))
  (import "env" "return_data"      (func $return_data      (param i32 i32)))
  (import "env" "storage_read"     (func $storage_read     (param i32 i32 i32) (result i32)))
  (import "env" "storage_write"    (func $storage_write    (param i32 i32 i32 i32)))
  (import "env" "storage_has"      (func $storage_has      (param i32 i32) (result i32)))
  (import "env" "storage_delete"   (func $storage_delete   (param i32 i32)))
  (import "env" "caller"           (func $caller           (param i32)))
  (import "env" "block_height"     (func $block_height     (result i64)))
  (import "env" "block_timestamp"  (func $block_timestamp  (result i64)))
  (import "env" "value_lo"         (func $value_lo         (result i64)))
  (import "env" "value_hi"         (func $value_hi         (result i64)))
  (import "env" "agent_get_score"  (func $agent_get_score  (param i32) (result i64)))
  (import "env" "agent_is_registered" (func $agent_is_registered (param i32) (result i32)))
  (import "env" "token_transfer"   (func $token_transfer   (param i32 i64 i64) (result i32)))
  (import "env" "log_msg"          (func $log_msg          (param i32 i32)))
  (import "env" "abort"            (func $abort            (param i32 i32)))

  (memory (export "memory") 1)

  ;; offset 0:  32-byte address buffer
  ;; offset 32: 8-byte lo result
  ;; offset 40: 8-byte hi result
  ;; offset 48: 16-byte return buffer (lo || hi, little-endian u128)

  (func (export "query_balance_u128")
    ;; get own address
    (call $contract_address (i32.const 0))
    ;; lo = token_balance(addr=0)
    (i64.store (i32.const 32) (call $token_balance (i32.const 0)))
    ;; hi = token_balance_hi(addr=0)
    (i64.store (i32.const 40) (call $token_balance_hi (i32.const 0)))
    ;; copy lo to return buf[0..8]
    (i64.store (i32.const 48) (i64.load (i32.const 32)))
    ;; copy hi to return buf[8..16]
    (i64.store (i32.const 56) (i64.load (i32.const 40)))
    ;; return_data(ptr=48, len=16)
    (call $return_data (i32.const 48) (i32.const 16))
  )
)
"#;

// ---------------------------------------------------------------------------
// Tests (written BEFORE implementation — RED phase)
// ---------------------------------------------------------------------------

/// Verify that `token_balance_hi` returns 0 for a balance that fits in u64.
#[test]
fn test_token_balance_hi_zero_for_small_balance() {
    let contract_addr = [1u8; 32];
    let balance: u128 = 42_000_000_000; // well within u64

    let mut balances = BTreeMap::new();
    balances.insert(contract_addr, balance);
    let chain = TestChainState { balances };

    let wasm = compile_wat(CONTRACT_BALANCE_FULL_U128);
    let engine = VmEngine::new();
    let result = engine
        .execute(
            &wasm,
            "query_balance_u128",
            &[],
            make_context(contract_addr),
            BTreeMap::new(),
            &chain,
        )
        .expect("execution must succeed");

    assert_eq!(result.return_data.len(), 16, "should return 16 bytes");
    let lo = u64::from_le_bytes(result.return_data[0..8].try_into().unwrap());
    let hi = u64::from_le_bytes(result.return_data[8..16].try_into().unwrap());
    let recovered: u128 = (hi as u128) << 64 | (lo as u128);

    assert_eq!(hi, 0, "hi word must be 0 for balance < 2^64");
    assert_eq!(recovered, balance, "recovered balance must equal original");
}

/// The critical regression test: balance > 2^64 must be fully recoverable.
///
/// Before the fix, `token_balance` silently truncated the high bits.
/// After the fix, calling `token_balance` + `token_balance_hi` gives the
/// full u128.
#[test]
fn test_token_balance_hi_large_balance_above_u64_max() {
    let contract_addr = [2u8; 32];
    // balance = 5 * 2^64 + 999  — high bits are 5, low bits are 999
    let hi_word: u64 = 5;
    let lo_word: u64 = 999;
    let balance: u128 = ((hi_word as u128) << 64) | (lo_word as u128);

    let mut balances = BTreeMap::new();
    balances.insert(contract_addr, balance);
    let chain = TestChainState { balances };

    let wasm = compile_wat(CONTRACT_BALANCE_FULL_U128);
    let engine = VmEngine::new();
    let result = engine
        .execute(
            &wasm,
            "query_balance_u128",
            &[],
            make_context(contract_addr),
            BTreeMap::new(),
            &chain,
        )
        .expect("execution must succeed");

    assert_eq!(result.return_data.len(), 16);
    let lo = u64::from_le_bytes(result.return_data[0..8].try_into().unwrap());
    let hi = u64::from_le_bytes(result.return_data[8..16].try_into().unwrap());
    let recovered: u128 = (hi as u128) << 64 | (lo as u128);

    assert_eq!(lo, lo_word, "lo word must match original low 64 bits");
    assert_eq!(hi, hi_word, "hi word must be non-zero for balance > 2^64");
    assert_eq!(recovered, balance, "full u128 balance must be exactly recovered");
}

/// Verify the extreme case: u128::MAX balance is fully recoverable.
#[test]
fn test_token_balance_hi_max_u128() {
    let contract_addr = [3u8; 32];
    let balance: u128 = u128::MAX;

    let mut balances = BTreeMap::new();
    balances.insert(contract_addr, balance);
    let chain = TestChainState { balances };

    let wasm = compile_wat(CONTRACT_BALANCE_FULL_U128);
    let engine = VmEngine::new();
    let result = engine
        .execute(
            &wasm,
            "query_balance_u128",
            &[],
            make_context(contract_addr),
            BTreeMap::new(),
            &chain,
        )
        .expect("execution must succeed");

    assert_eq!(result.return_data.len(), 16);
    let lo = u64::from_le_bytes(result.return_data[0..8].try_into().unwrap());
    let hi = u64::from_le_bytes(result.return_data[8..16].try_into().unwrap());
    let recovered: u128 = (hi as u128) << 64 | (lo as u128);

    assert_eq!(lo, u64::MAX);
    assert_eq!(hi, u64::MAX);
    assert_eq!(recovered, u128::MAX);
}

/// Backward-compatibility: old contracts that only use `token_balance` (lo)
/// must still compile and execute without errors. They simply won't see the
/// high bits — but they must not crash or error.
#[test]
fn test_backward_compat_lo_only_contract_still_works() {
    let contract_addr = [4u8; 32];
    let balance: u128 = 100_000; // small balance that fits in u64

    let mut balances = BTreeMap::new();
    balances.insert(contract_addr, balance);
    let chain = TestChainState { balances };

    let wasm = compile_wat(CONTRACT_BALANCE_LO_ONLY);
    let engine = VmEngine::new();
    let result = engine
        .execute(
            &wasm,
            "query_balance_lo",
            &[],
            make_context(contract_addr),
            BTreeMap::new(),
            &chain,
        )
        .expect("legacy lo-only contract must still execute without error");

    assert_eq!(result.return_data.len(), 8, "lo-only contract returns 8 bytes");
    let lo = u64::from_le_bytes(result.return_data[0..8].try_into().unwrap());
    assert_eq!(lo as u128, balance, "lo word matches balance when balance < 2^64");
}

/// Address with no balance entry returns 0 for both lo and hi.
#[test]
fn test_token_balance_hi_missing_address_returns_zero() {
    let contract_addr = [5u8; 32];
    // balances map is empty — address not present
    let chain = TestChainState {
        balances: BTreeMap::new(),
    };

    let wasm = compile_wat(CONTRACT_BALANCE_FULL_U128);
    let engine = VmEngine::new();
    let result = engine
        .execute(
            &wasm,
            "query_balance_u128",
            &[],
            make_context(contract_addr),
            BTreeMap::new(),
            &chain,
        )
        .expect("execution must succeed even for unknown address");

    assert_eq!(result.return_data.len(), 16);
    let lo = u64::from_le_bytes(result.return_data[0..8].try_into().unwrap());
    let hi = u64::from_le_bytes(result.return_data[8..16].try_into().unwrap());
    assert_eq!(lo, 0);
    assert_eq!(hi, 0);
}
