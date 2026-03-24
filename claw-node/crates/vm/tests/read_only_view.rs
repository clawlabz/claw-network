//! TDD tests for read-only (view) call enforcement.
//!
//! Tests are written BEFORE implementation (RED phase).
//!
//! Spec:
//!   - ExecutionContext gains a `read_only: bool` field.
//!   - host_storage_write, host_storage_delete, host_token_transfer trap with
//!     "write operation not allowed in read-only (view) call" when read_only=true.
//!   - host_storage_read succeeds in read-only mode.
//!   - VIEW_CALL_FUEL_LIMIT = 5_000_000 (half of DEFAULT_FUEL_LIMIT = 10_000_000).
//!   - Existing callers with read_only=false behave exactly as before.

use std::collections::BTreeMap;

use claw_vm::{ChainState, ExecutionContext, VmEngine, DEFAULT_FUEL_LIMIT, VIEW_CALL_FUEL_LIMIT};

// ---------------------------------------------------------------------------
// Minimal ChainState stub
// ---------------------------------------------------------------------------

struct TestChainState;

impl ChainState for TestChainState {
    fn get_balance(&self, _: &[u8; 32]) -> u128 {
        0
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
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn make_context_rw() -> ExecutionContext {
    ExecutionContext {
        caller: [0u8; 32],
        contract_address: [1u8; 32],
        block_height: 1,
        block_timestamp: 0,
        value: 0,
        fuel_limit: DEFAULT_FUEL_LIMIT,
        read_only: false,
    }
}

fn make_context_view() -> ExecutionContext {
    ExecutionContext {
        caller: [0u8; 32],
        contract_address: [1u8; 32],
        block_height: 1,
        block_timestamp: 0,
        value: 0,
        fuel_limit: VIEW_CALL_FUEL_LIMIT,
        read_only: true,
    }
}

fn compile_wat(src: &str) -> Vec<u8> {
    wat::parse_str(src).expect("WAT compilation failed")
}

// ---------------------------------------------------------------------------
// Minimal WAT contract that imports every host function (required by Wasmer's
// strict import matching): contracts must declare exactly the imports the
// engine provides.  Each exported method exercises one host function.
// ---------------------------------------------------------------------------

/// A WAT module that imports all host functions and exports isolated test methods.
const CONTRACT_ALL_OPS: &str = r#"
(module
  (import "env" "storage_read"        (func $storage_read     (param i32 i32 i32) (result i32)))
  (import "env" "storage_write"       (func $storage_write    (param i32 i32 i32 i32)))
  (import "env" "storage_has"         (func $storage_has      (param i32 i32) (result i32)))
  (import "env" "storage_delete"      (func $storage_delete   (param i32 i32)))
  (import "env" "caller"              (func $caller           (param i32)))
  (import "env" "block_height"        (func $block_height     (result i64)))
  (import "env" "block_timestamp"     (func $block_timestamp  (result i64)))
  (import "env" "contract_address"    (func $contract_address (param i32)))
  (import "env" "value_lo"            (func $value_lo         (result i64)))
  (import "env" "value_hi"            (func $value_hi         (result i64)))
  (import "env" "agent_get_score"     (func $agent_get_score  (param i32) (result i64)))
  (import "env" "agent_is_registered" (func $agent_is_registered (param i32) (result i32)))
  (import "env" "token_balance"       (func $token_balance    (param i32) (result i64)))
  (import "env" "token_balance_hi"    (func $token_balance_hi (param i32) (result i64)))
  (import "env" "token_transfer"      (func $token_transfer   (param i32 i64 i64) (result i32)))
  (import "env" "log_msg"             (func $log_msg          (param i32 i32)))
  (import "env" "return_data"         (func $return_data      (param i32 i32)))
  (import "env" "abort"               (func $abort            (param i32 i32)))

  (memory (export "memory") 1)

  ;; mem[0..32]  = key  "k" (1 byte used)
  ;; mem[32..64] = val  "v" (1 byte used)
  ;; mem[64..96] = read-back buffer

  (data (i32.const 0)  "k")
  (data (i32.const 32) "v")

  ;; --- write: calls storage_write(key=0/1, val=32/1) ---
  (func (export "do_write")
    (call $storage_write (i32.const 0) (i32.const 1) (i32.const 32) (i32.const 1))
  )

  ;; --- delete: calls storage_delete(key=0/1) ---
  (func (export "do_delete")
    (call $storage_delete (i32.const 0) (i32.const 1))
  )

  ;; --- transfer: calls token_transfer(addr=0, lo=1, hi=0) ---
  (func (export "do_transfer")
    (drop (call $token_transfer (i32.const 0) (i64.const 1) (i64.const 0)))
  )

  ;; --- read: calls storage_read(key=0/1, val_ptr=64) --- returns length as return_data ---
  (func (export "do_read")
    (local $len i32)
    (local.set $len (call $storage_read (i32.const 0) (i32.const 1) (i32.const 64)))
    ;; store len as 4-byte LE at mem[96]
    (i32.store (i32.const 96) (local.get $len))
    (call $return_data (i32.const 96) (i32.const 4))
  )
)
"#;

// ---------------------------------------------------------------------------
// Test 1: Normal (read_only=false) storage_write succeeds
// ---------------------------------------------------------------------------

#[test]
fn test_normal_call_storage_write_succeeds() {
    let wasm = compile_wat(CONTRACT_ALL_OPS);
    let engine = VmEngine::new();
    let result = engine.execute(
        &wasm,
        "do_write",
        &[],
        make_context_rw(),
        BTreeMap::new(),
        &TestChainState,
    );
    assert!(result.is_ok(), "write in normal call must succeed, got: {:?}", result);
    let r = result.unwrap();
    assert_eq!(r.storage_changes.len(), 1, "must record one storage change");
}

// ---------------------------------------------------------------------------
// Test 2: View call (read_only=true) storage_write traps
// ---------------------------------------------------------------------------

#[test]
fn test_view_call_storage_write_traps() {
    let wasm = compile_wat(CONTRACT_ALL_OPS);
    let engine = VmEngine::new();
    let result = engine.execute(
        &wasm,
        "do_write",
        &[],
        make_context_view(),
        BTreeMap::new(),
        &TestChainState,
    );
    assert!(result.is_err(), "write in view call must trap");
    let msg = result.unwrap_err().to_string();
    assert!(
        msg.contains("write operation not allowed in read-only"),
        "trap message must mention read-only, got: {msg}"
    );
}

// ---------------------------------------------------------------------------
// Test 3: View call storage_delete traps
// ---------------------------------------------------------------------------

#[test]
fn test_view_call_storage_delete_traps() {
    // Pre-populate storage so delete has something to delete
    let mut storage = BTreeMap::new();
    storage.insert(b"k".to_vec(), b"v".to_vec());

    let wasm = compile_wat(CONTRACT_ALL_OPS);
    let engine = VmEngine::new();
    let result = engine.execute(
        &wasm,
        "do_delete",
        &[],
        make_context_view(),
        storage,
        &TestChainState,
    );
    assert!(result.is_err(), "delete in view call must trap");
    let msg = result.unwrap_err().to_string();
    assert!(
        msg.contains("write operation not allowed in read-only"),
        "trap message must mention read-only, got: {msg}"
    );
}

// ---------------------------------------------------------------------------
// Test 4: View call token_transfer traps
// ---------------------------------------------------------------------------

#[test]
fn test_view_call_token_transfer_traps() {
    let wasm = compile_wat(CONTRACT_ALL_OPS);
    let engine = VmEngine::new();
    let result = engine.execute(
        &wasm,
        "do_transfer",
        &[],
        make_context_view(),
        BTreeMap::new(),
        &TestChainState,
    );
    assert!(result.is_err(), "token_transfer in view call must trap");
    let msg = result.unwrap_err().to_string();
    assert!(
        msg.contains("write operation not allowed in read-only"),
        "trap message must mention read-only, got: {msg}"
    );
}

// ---------------------------------------------------------------------------
// Test 5: View call storage_read succeeds (reads are allowed)
// ---------------------------------------------------------------------------

#[test]
fn test_view_call_storage_read_succeeds() {
    let mut storage = BTreeMap::new();
    storage.insert(b"k".to_vec(), b"v".to_vec());

    let wasm = compile_wat(CONTRACT_ALL_OPS);
    let engine = VmEngine::new();
    let result = engine.execute(
        &wasm,
        "do_read",
        &[],
        make_context_view(),
        storage,
        &TestChainState,
    );
    assert!(result.is_ok(), "storage_read in view call must succeed, got: {:?}", result);
    let r = result.unwrap();
    assert_eq!(r.storage_changes.len(), 0, "view call must produce no storage changes");
    // return_data is 4-byte LE i32 of the length (1 for "v")
    assert_eq!(r.return_data.len(), 4);
    let len = i32::from_le_bytes(r.return_data[..4].try_into().unwrap());
    assert_eq!(len, 1, "storage_read should return length 1 for value 'v'");
}

// ---------------------------------------------------------------------------
// Test 6: VIEW_CALL_FUEL_LIMIT constant is exactly 5_000_000
// ---------------------------------------------------------------------------

#[test]
fn test_view_call_fuel_limit_is_5_million() {
    assert_eq!(VIEW_CALL_FUEL_LIMIT, 5_000_000, "VIEW_CALL_FUEL_LIMIT must be 5_000_000");
    assert_eq!(DEFAULT_FUEL_LIMIT, 10_000_000, "DEFAULT_FUEL_LIMIT must be 10_000_000");
    assert_eq!(
        VIEW_CALL_FUEL_LIMIT,
        DEFAULT_FUEL_LIMIT / 2,
        "VIEW_CALL_FUEL_LIMIT must be half of DEFAULT_FUEL_LIMIT"
    );
}

// ---------------------------------------------------------------------------
// Test 7: Existing tests regression — read_only defaults to false
// ---------------------------------------------------------------------------

/// Verifies that omitting read_only=false still compiles (struct literal).
/// This guards the existing callers in handlers.rs and chain.rs.
#[test]
fn test_read_only_defaults_to_false_semantically() {
    // The context built by existing callers sets read_only: false.
    // A write operation must succeed.
    let ctx = ExecutionContext {
        caller: [0u8; 32],
        contract_address: [1u8; 32],
        block_height: 1,
        block_timestamp: 0,
        value: 0,
        fuel_limit: DEFAULT_FUEL_LIMIT,
        read_only: false, // explicit — mirrors what existing callers set
    };

    let wasm = compile_wat(CONTRACT_ALL_OPS);
    let engine = VmEngine::new();
    let result = engine.execute(&wasm, "do_write", &[], ctx, BTreeMap::new(), &TestChainState);
    assert!(
        result.is_ok(),
        "write with read_only=false must succeed (regression guard)"
    );
}
