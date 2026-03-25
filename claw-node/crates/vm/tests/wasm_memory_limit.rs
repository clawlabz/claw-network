//! Integration tests for Wasm memory page limit enforcement.
//!
//! The VM must reject any contract whose memory declaration exceeds
//! `MAX_WASM_MEMORY_PAGES` (256 pages = 16 MB).

use std::collections::BTreeMap;

use claw_vm::{ChainState, ExecutionContext, VmError, VmEngine, MAX_WASM_MEMORY_PAGES};

// ---------------------------------------------------------------------------
// Minimal ChainState stub
// ---------------------------------------------------------------------------

struct EmptyChainState;

impl ChainState for EmptyChainState {
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
    fn get_contract_code(&self, _contract: &[u8; 32]) -> Option<Vec<u8>> {
        None
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn default_context() -> ExecutionContext {
    ExecutionContext::new_top_level(
        [0u8; 32],
        [1u8; 32],
        1,
        0,
        0,
        claw_vm::DEFAULT_FUEL_LIMIT,
        false,
    )
}

fn compile_wat(src: &str) -> Vec<u8> {
    wat::parse_str(src).expect("WAT compilation failed")
}

// ---------------------------------------------------------------------------
// WAT contracts
// ---------------------------------------------------------------------------

/// A contract with initial memory of 300 pages — exceeds the 256-page limit.
const CONTRACT_MEMORY_TOO_LARGE: &str = r#"
(module
  (import "env" "storage_read"     (func $storage_read     (param i32 i32 i32) (result i32)))
  (import "env" "storage_write"    (func $storage_write    (param i32 i32 i32 i32)))
  (import "env" "storage_has"      (func $storage_has      (param i32 i32) (result i32)))
  (import "env" "storage_delete"   (func $storage_delete   (param i32 i32)))
  (import "env" "caller"           (func $caller           (param i32)))
  (import "env" "block_height"     (func $block_height     (result i64)))
  (import "env" "block_timestamp"  (func $block_timestamp  (result i64)))
  (import "env" "contract_address" (func $contract_address (param i32)))
  (import "env" "value_lo"         (func $value_lo         (result i64)))
  (import "env" "value_hi"         (func $value_hi         (result i64)))
  (import "env" "agent_get_score"  (func $agent_get_score  (param i32) (result i64)))
  (import "env" "agent_is_registered" (func $agent_is_registered (param i32) (result i32)))
  (import "env" "token_balance"    (func $token_balance    (param i32) (result i64)))
  (import "env" "token_balance_hi" (func $token_balance_hi (param i32) (result i64)))
  (import "env" "token_transfer"   (func $token_transfer   (param i32 i64 i64) (result i32)))
  (import "env" "log_msg"          (func $log_msg          (param i32 i32)))
  (import "env" "return_data"      (func $return_data      (param i32 i32)))
  (import "env" "abort"            (func $abort            (param i32 i32)))
  (import "env" "emit_event"       (func $emit_event       (param i32 i32 i32 i32)))
  (import "env" "call_contract"    (func $call_contract    (param i32 i32 i32 i32 i32 i64 i64) (result i32)))
  (import "env" "cross_call_return_data" (func $cross_call_return_data (param i32) (result i32)))

  (memory (export "memory") 300)

  (func (export "run")
    nop
  )
)
"#;

/// A contract with max memory of 512 pages — exceeds the 256-page limit even
/// though the initial size (1 page) is fine.
const CONTRACT_MEMORY_MAX_TOO_LARGE: &str = r#"
(module
  (import "env" "storage_read"     (func $storage_read     (param i32 i32 i32) (result i32)))
  (import "env" "storage_write"    (func $storage_write    (param i32 i32 i32 i32)))
  (import "env" "storage_has"      (func $storage_has      (param i32 i32) (result i32)))
  (import "env" "storage_delete"   (func $storage_delete   (param i32 i32)))
  (import "env" "caller"           (func $caller           (param i32)))
  (import "env" "block_height"     (func $block_height     (result i64)))
  (import "env" "block_timestamp"  (func $block_timestamp  (result i64)))
  (import "env" "contract_address" (func $contract_address (param i32)))
  (import "env" "value_lo"         (func $value_lo         (result i64)))
  (import "env" "value_hi"         (func $value_hi         (result i64)))
  (import "env" "agent_get_score"  (func $agent_get_score  (param i32) (result i64)))
  (import "env" "agent_is_registered" (func $agent_is_registered (param i32) (result i32)))
  (import "env" "token_balance"    (func $token_balance    (param i32) (result i64)))
  (import "env" "token_balance_hi" (func $token_balance_hi (param i32) (result i64)))
  (import "env" "token_transfer"   (func $token_transfer   (param i32 i64 i64) (result i32)))
  (import "env" "log_msg"          (func $log_msg          (param i32 i32)))
  (import "env" "return_data"      (func $return_data      (param i32 i32)))
  (import "env" "abort"            (func $abort            (param i32 i32)))
  (import "env" "emit_event"       (func $emit_event       (param i32 i32 i32 i32)))
  (import "env" "call_contract"    (func $call_contract    (param i32 i32 i32 i32 i32 i64 i64) (result i32)))
  (import "env" "cross_call_return_data" (func $cross_call_return_data (param i32) (result i32)))

  (memory (export "memory") 1 512)

  (func (export "run")
    nop
  )
)
"#;

/// A contract with memory within the limit (1 page initial, 256 max).
const CONTRACT_MEMORY_WITHIN_LIMIT: &str = r#"
(module
  (import "env" "storage_read"     (func $storage_read     (param i32 i32 i32) (result i32)))
  (import "env" "storage_write"    (func $storage_write    (param i32 i32 i32 i32)))
  (import "env" "storage_has"      (func $storage_has      (param i32 i32) (result i32)))
  (import "env" "storage_delete"   (func $storage_delete   (param i32 i32)))
  (import "env" "caller"           (func $caller           (param i32)))
  (import "env" "block_height"     (func $block_height     (result i64)))
  (import "env" "block_timestamp"  (func $block_timestamp  (result i64)))
  (import "env" "contract_address" (func $contract_address (param i32)))
  (import "env" "value_lo"         (func $value_lo         (result i64)))
  (import "env" "value_hi"         (func $value_hi         (result i64)))
  (import "env" "agent_get_score"  (func $agent_get_score  (param i32) (result i64)))
  (import "env" "agent_is_registered" (func $agent_is_registered (param i32) (result i32)))
  (import "env" "token_balance"    (func $token_balance    (param i32) (result i64)))
  (import "env" "token_balance_hi" (func $token_balance_hi (param i32) (result i64)))
  (import "env" "token_transfer"   (func $token_transfer   (param i32 i64 i64) (result i32)))
  (import "env" "log_msg"          (func $log_msg          (param i32 i32)))
  (import "env" "return_data"      (func $return_data      (param i32 i32)))
  (import "env" "abort"            (func $abort            (param i32 i32)))
  (import "env" "emit_event"       (func $emit_event       (param i32 i32 i32 i32)))
  (import "env" "call_contract"    (func $call_contract    (param i32 i32 i32 i32 i32 i64 i64) (result i32)))
  (import "env" "cross_call_return_data" (func $cross_call_return_data (param i32) (result i32)))

  (memory (export "memory") 1 256)

  (func (export "run")
    nop
  )
)
"#;

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// Contract declaring 300 initial pages must be rejected with MemoryLimitExceeded.
#[test]
fn test_memory_initial_pages_exceed_limit() {
    let wasm = compile_wat(CONTRACT_MEMORY_TOO_LARGE);
    let engine = VmEngine::new();
    let chain = EmptyChainState;

    let err = engine
        .execute(
            &wasm,
            "run",
            &[],
            default_context(),
            BTreeMap::new(),
            &chain,
        )
        .expect_err("contract with 300 initial pages must be rejected");

    match err {
        VmError::MemoryLimitExceeded { pages, max } => {
            assert_eq!(pages, 300, "reported pages must be 300");
            assert_eq!(max, MAX_WASM_MEMORY_PAGES, "max must match the constant");
        }
        other => panic!("expected VmError::MemoryLimitExceeded, got: {other:?}"),
    }
}

/// Contract declaring max 512 pages must be rejected even if initial is 1.
#[test]
fn test_memory_max_pages_exceed_limit() {
    let wasm = compile_wat(CONTRACT_MEMORY_MAX_TOO_LARGE);
    let engine = VmEngine::new();
    let chain = EmptyChainState;

    let err = engine
        .execute(
            &wasm,
            "run",
            &[],
            default_context(),
            BTreeMap::new(),
            &chain,
        )
        .expect_err("contract with max 512 pages must be rejected");

    match err {
        VmError::MemoryLimitExceeded { pages, max } => {
            assert_eq!(pages, 512, "reported pages must be 512");
            assert_eq!(max, MAX_WASM_MEMORY_PAGES);
        }
        other => panic!("expected VmError::MemoryLimitExceeded, got: {other:?}"),
    }
}

/// Contract within the limit (1 initial, 256 max) must execute successfully.
#[test]
fn test_memory_within_limit_succeeds() {
    let wasm = compile_wat(CONTRACT_MEMORY_WITHIN_LIMIT);
    let engine = VmEngine::new();
    let chain = EmptyChainState;

    let result = engine.execute(
        &wasm,
        "run",
        &[],
        default_context(),
        BTreeMap::new(),
        &chain,
    );

    assert!(result.is_ok(), "contract within memory limit must succeed: {result:?}");
}

/// validate() must also reject contracts exceeding the memory limit.
#[test]
fn test_validate_rejects_excess_memory() {
    let wasm = compile_wat(CONTRACT_MEMORY_TOO_LARGE);
    let engine = VmEngine::new();

    let err = engine.validate(&wasm).expect_err("validate must reject 300-page contract");

    match err {
        VmError::MemoryLimitExceeded { pages, max } => {
            assert_eq!(pages, 300);
            assert_eq!(max, MAX_WASM_MEMORY_PAGES);
        }
        other => panic!("expected VmError::MemoryLimitExceeded, got: {other:?}"),
    }
}
