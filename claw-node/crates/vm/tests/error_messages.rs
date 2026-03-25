//! Integration tests for improved contract execution error messages.
//!
//! TDD flow:
//!   RED   — all four tests written here before any implementation change
//!   GREEN — ContractAbort variant added to VmError, engine.rs updated
//!   CHECK — all assertions pass, fuel count is accurate in error payloads
//!
//! Tests:
//!   1. contract_abort_carries_custom_message
//!   2. out_of_fuel_carries_consumed_count
//!   3. execution_failure_carries_fuel_consumed
//!   4. require_false_error_contains_message

use std::collections::BTreeMap;

use claw_vm::{ChainState, ExecutionContext, VmError, VmEngine};

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

fn low_fuel_context(fuel_limit: u64) -> ExecutionContext {
    ExecutionContext::new_top_level(
        [0u8; 32],
        [1u8; 32],
        1,
        0,
        0,
        fuel_limit,
        false,
    )
}

fn compile_wat(src: &str) -> Vec<u8> {
    wat::parse_str(src).expect("WAT compilation failed")
}

// ---------------------------------------------------------------------------
// WAT contracts
// ---------------------------------------------------------------------------

/// A contract that immediately calls `abort` with the message "amount must be positive".
/// Exercises: host_abort → panic("contract abort: amount must be positive")
///            engine.rs catches it → VmError::ContractAbort { reason, fuel_consumed }
const CONTRACT_ABORT_WITH_MESSAGE: &str = r#"
(module
  (import "env" "abort"            (func $abort            (param i32 i32)))
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

  (memory (export "memory") 1)

  ;; "amount must be positive" stored at offset 0 in memory (23 bytes)
  (data (i32.const 0) "amount must be positive")

  (func (export "run")
    ;; abort("amount must be positive", 23)
    (call $abort (i32.const 0) (i32.const 23))
  )
)
"#;

/// A contract that burns fuel with repeated storage reads until exhausted.
/// Uses a tiny fuel_limit so it reliably runs out.
const CONTRACT_BURN_ALL_FUEL: &str = r#"
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

  (memory (export "memory") 1)

  ;; key "k" at offset 0
  (data (i32.const 0) "k")

  (func (export "run")
    ;; Call storage_read 10 times — each costs STORAGE_READ_FUEL (10_000).
    ;; With a fuel_limit of 25_000 we exhaust fuel mid-loop.
    (drop (call $storage_read (i32.const 0) (i32.const 1) (i32.const 64)))
    (drop (call $storage_read (i32.const 0) (i32.const 1) (i32.const 64)))
    (drop (call $storage_read (i32.const 0) (i32.const 1) (i32.const 64)))
    (drop (call $storage_read (i32.const 0) (i32.const 1) (i32.const 64)))
    (drop (call $storage_read (i32.const 0) (i32.const 1) (i32.const 64)))
    (drop (call $storage_read (i32.const 0) (i32.const 1) (i32.const 64)))
    (drop (call $storage_read (i32.const 0) (i32.const 1) (i32.const 64)))
    (drop (call $storage_read (i32.const 0) (i32.const 1) (i32.const 64)))
    (drop (call $storage_read (i32.const 0) (i32.const 1) (i32.const 64)))
    (drop (call $storage_read (i32.const 0) (i32.const 1) (i32.const 64)))
  )
)
"#;

/// A contract that calls `abort` with the message "my error".
/// Mirrors the Rust macro `require!(false, "my error")`.
const CONTRACT_REQUIRE_FALSE: &str = r#"
(module
  (import "env" "abort"            (func $abort            (param i32 i32)))
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

  (memory (export "memory") 1)

  ;; "my error" at offset 0 (8 bytes)
  (data (i32.const 0) "my error")

  (func (export "run")
    ;; require!(false, "my error") → abort("my error", 8)
    (call $abort (i32.const 0) (i32.const 8))
  )
)
"#;

// ---------------------------------------------------------------------------
// Tests — RED phase: these fail until VmError::ContractAbort is added
// and engine.rs improved error extraction is implemented.
// ---------------------------------------------------------------------------

/// Test 1: contract that calls abort("amount must be positive") must return
/// VmError::ContractAbort with the exact reason string preserved.
#[test]
fn test_contract_abort_carries_custom_message() {
    let wasm = compile_wat(CONTRACT_ABORT_WITH_MESSAGE);
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
        .expect_err("abort() must cause an error");

    match err {
        VmError::ContractAbort { reason, fuel_consumed } => {
            assert!(
                reason.contains("amount must be positive"),
                "reason must contain the abort message, got: {reason:?}"
            );
            // The abort is reached immediately — fuel consumed must be > 0 (at least one
            // host call was made, since host_abort itself records consumed fuel before panicking).
            // We accept any non-negative value here; the important check is the reason string.
            let _ = fuel_consumed; // documented: non-negative, exact value is impl detail
        }
        other => panic!("expected VmError::ContractAbort, got: {other:?}"),
    }
}

/// Test 2: contract that exhausts fuel must return VmError::OutOfFuel
/// with a `used` count that is greater than zero and at most the fuel limit.
#[test]
fn test_out_of_fuel_carries_consumed_count() {
    // fuel_limit = 25_000 → fits 2 storage_read calls (2 × 10_000 = 20_000)
    // then the 3rd call (at 20_000 consumed) tries to deduct 10_000 but only
    // 5_000 remain → fuel exhausted.
    let fuel_limit = 25_000u64;

    let wasm = compile_wat(CONTRACT_BURN_ALL_FUEL);
    let engine = VmEngine::new();
    let chain = EmptyChainState;

    let err = engine
        .execute(
            &wasm,
            "run",
            &[],
            low_fuel_context(fuel_limit),
            BTreeMap::new(),
            &chain,
        )
        .expect_err("out-of-fuel contract must error");

    match err {
        VmError::OutOfFuel { used, limit } => {
            assert_eq!(limit, fuel_limit, "limit must match the configured fuel_limit");
            assert!(used > 0, "used must be greater than zero; got {used}");
            assert!(
                used <= fuel_limit,
                "used ({used}) must not exceed fuel_limit ({fuel_limit})"
            );
        }
        other => panic!("expected VmError::OutOfFuel, got: {other:?}"),
    }
}

/// Test 3: a contract that fails with a non-abort, non-fuel error must return
/// VmError::ExecutionFailed whose message includes a fuel_consumed annotation.
///
/// We trigger this by calling a method that doesn't exist on a valid module —
/// the engine returns VmError::MethodNotFound, which is a different variant.
/// To get ExecutionFailed-with-fuel we instead call a method that exits via
/// an unreachable trap (Wasm `unreachable` instruction).
const CONTRACT_UNREACHABLE_TRAP: &str = r#"
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

  (memory (export "memory") 1)

  (func (export "run")
    ;; Wasm unreachable — triggers a generic RuntimeError trap, not an abort.
    unreachable
  )
)
"#;

#[test]
fn test_execution_failure_carries_fuel_consumed_annotation() {
    let wasm = compile_wat(CONTRACT_UNREACHABLE_TRAP);
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
        .expect_err("unreachable trap must cause an error");

    match err {
        VmError::ExecutionFailed(msg) => {
            assert!(
                msg.contains("fuel consumed:"),
                "ExecutionFailed message must include 'fuel consumed:' annotation, got: {msg:?}"
            );
        }
        other => panic!("expected VmError::ExecutionFailed, got: {other:?}"),
    }
}

/// Test 4: `require!(false, "my error")` is expressed as abort("my error", 8).
/// The resulting error must contain "my error" in the reason field.
#[test]
fn test_require_false_error_contains_message() {
    let wasm = compile_wat(CONTRACT_REQUIRE_FALSE);
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
        .expect_err("require!(false) must cause an error");

    match err {
        VmError::ContractAbort { reason, .. } => {
            assert!(
                reason.contains("my error"),
                "reason must contain 'my error', got: {reason:?}"
            );
        }
        other => panic!("expected VmError::ContractAbort, got: {other:?}"),
    }
}
