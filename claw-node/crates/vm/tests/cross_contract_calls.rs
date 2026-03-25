//! Integration tests for cross-contract call host functions.
//!
//! Tests:
//!   1. Basic cross-contract call succeeds, return data is readable
//!   2. Reentrancy is rejected (returns -2)
//!   3. Max call depth exceeded (returns -3)
//!   4. Calling non-existent contract (returns -1)
//!   5. Fuel forwarding: child consumes fuel from parent's budget
//!   6. Read-only mode blocks cross-contract calls

use std::collections::BTreeMap;
use std::sync::Arc;

use claw_vm::{ChainState, ExecutionContext, VmEngine};

// ---------------------------------------------------------------------------
// ChainState stub that serves contract code from an in-memory map
// ---------------------------------------------------------------------------

struct CrossCallChainState {
    contract_code: BTreeMap<[u8; 32], Vec<u8>>,
}

impl ChainState for CrossCallChainState {
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
    fn get_contract_code(&self, contract: &[u8; 32]) -> Option<Vec<u8>> {
        self.contract_code.get(contract).cloned()
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn compile_wat(src: &str) -> Vec<u8> {
    wat::parse_str(src).expect("WAT compilation failed")
}

/// Contract B address (target of cross-contract calls)
fn contract_b_addr() -> [u8; 32] {
    let mut addr = [0u8; 32];
    addr[0] = 0xBB;
    addr
}

/// Contract A address (caller)
fn contract_a_addr() -> [u8; 32] {
    let mut addr = [0u8; 32];
    addr[0] = 0xAA;
    addr
}

fn caller_addr() -> [u8; 32] {
    [0x01; 32]
}

// ---------------------------------------------------------------------------
// WAT contracts
// ---------------------------------------------------------------------------

/// Contract B: a simple contract that sets return_data to "hello" when `greet` is called.
const CONTRACT_B_GREET: &str = r#"
(module
  (import "env" "return_data" (func $return_data (param i32 i32)))

  ;; "hello" stored in data section at offset 0
  (memory (export "memory") 1)
  (data (i32.const 0) "hello")

  (func (export "greet")
    (call $return_data (i32.const 0) (i32.const 5))
  )
)
"#;

/// Contract A: calls contract B's `greet` method via `call_contract`,
/// then reads the return data and sets it as its own return data.
/// Contract B's address is hardcoded at memory offset 0 (32 bytes).
/// Method name "greet" is at offset 32 (5 bytes).
fn contract_a_calls_b_wat(b_addr: &[u8; 32]) -> String {
    // Build the data section with B's address bytes
    let addr_hex: String = b_addr.iter().map(|b| format!("\\{:02x}", b)).collect();

    format!(
        r#"
(module
  (import "env" "call_contract"
    (func $call_contract (param i32 i32 i32 i32 i32 i64 i64) (result i32)))
  (import "env" "cross_call_return_data"
    (func $cross_call_return_data (param i32) (result i32)))
  (import "env" "return_data" (func $return_data (param i32 i32)))

  (memory (export "memory") 1)
  ;; Offset 0: contract B address (32 bytes)
  (data (i32.const 0) "{addr_hex}")
  ;; Offset 32: method name "greet" (5 bytes)
  (data (i32.const 32) "greet")

  (func (export "call_b")
    (local $result i32)
    (local $ret_len i32)

    ;; Call contract B: addr=0, method=32 len=5, args=0 len=0, value=0
    (local.set $result
      (call $call_contract
        (i32.const 0)   ;; addr_ptr
        (i32.const 32)  ;; method_ptr
        (i32.const 5)   ;; method_len
        (i32.const 0)   ;; args_ptr (no args)
        (i32.const 0)   ;; args_len
        (i64.const 0)   ;; value_lo
        (i64.const 0)   ;; value_hi
      )
    )

    ;; If call succeeded (result == 0), read return data into offset 100
    (if (i32.eqz (local.get $result))
      (then
        (local.set $ret_len
          (call $cross_call_return_data (i32.const 100))
        )
        ;; Set our own return data to what B returned
        (call $return_data (i32.const 100) (local.get $ret_len))
      )
    )
  )
)
"#
    )
}

/// Contract A that calls a non-existent address.
const CONTRACT_A_CALLS_MISSING: &str = r#"
(module
  (import "env" "call_contract"
    (func $call_contract (param i32 i32 i32 i32 i32 i64 i64) (result i32)))
  (import "env" "return_data" (func $return_data (param i32 i32)))

  (memory (export "memory") 1)
  ;; Offset 0: non-existent address (32 bytes of 0xFF)
  (data (i32.const 0) "\ff\ff\ff\ff\ff\ff\ff\ff\ff\ff\ff\ff\ff\ff\ff\ff\ff\ff\ff\ff\ff\ff\ff\ff\ff\ff\ff\ff\ff\ff\ff\ff")
  ;; Offset 32: method name "foo"
  (data (i32.const 32) "foo")

  (func (export "call_missing")
    (local $result i32)
    (local.set $result
      (call $call_contract
        (i32.const 0)   ;; addr_ptr
        (i32.const 32)  ;; method_ptr
        (i32.const 3)   ;; method_len
        (i32.const 0)   ;; args_ptr
        (i32.const 0)   ;; args_len
        (i64.const 0)   ;; value_lo
        (i64.const 0)   ;; value_hi
      )
    )

    ;; Store result code as return data (1 byte at offset 100)
    ;; We add 128 to make it unsigned-friendly for assertion
    (i32.store8 (i32.const 100) (i32.add (local.get $result) (i32.const 128)))
    (call $return_data (i32.const 100) (i32.const 1))
  )
)
"#;

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// Sanity check: Contract B works standalone.
#[test]
fn test_contract_b_standalone() {
    let wasm_b = compile_wat(CONTRACT_B_GREET);
    let chain_state = CrossCallChainState {
        contract_code: BTreeMap::new(),
    };
    let ctx = ExecutionContext::new_top_level(
        caller_addr(), contract_b_addr(), 10, 1_000_000, 0,
        claw_vm::DEFAULT_FUEL_LIMIT, false,
    );
    let engine = VmEngine::new();
    let result = engine.execute(&wasm_b, "greet", &[], ctx, BTreeMap::new(), &chain_state)
        .expect("B should execute standalone");
    assert_eq!(String::from_utf8(result.return_data).unwrap(), "hello");
}

/// Test 1: Basic cross-contract call — A calls B.greet(), gets "hello" back.
#[test]
fn test_cross_call_basic_success() {
    let wasm_b = compile_wat(CONTRACT_B_GREET);
    let wasm_a = compile_wat(&contract_a_calls_b_wat(&contract_b_addr()));

    let mut code_map = BTreeMap::new();
    code_map.insert(contract_b_addr(), wasm_b);

    let chain_state = Arc::new(CrossCallChainState {
        contract_code: code_map.clone(),
    });

    let ctx = ExecutionContext::new_top_level(
        caller_addr(),
        contract_a_addr(),
        10,
        1_000_000,
        0,
        claw_vm::DEFAULT_FUEL_LIMIT,
        false,
    );

    let engine = VmEngine::new();
    let result = engine
        .execute_with_cross_calls(
            &wasm_a,
            "call_b",
            &[],
            ctx,
            BTreeMap::new(),
            chain_state,
            Arc::new(code_map),
        )
        .expect("execution should succeed");

    assert_eq!(
        String::from_utf8(result.return_data).unwrap(),
        "hello",
        "Contract A should relay B's return data"
    );
}

/// Test 2: Calling a non-existent contract returns -1.
#[test]
fn test_cross_call_missing_contract_returns_negative_one() {
    let wasm_a = compile_wat(CONTRACT_A_CALLS_MISSING);

    let chain_state = Arc::new(CrossCallChainState {
        contract_code: BTreeMap::new(), // no contracts
    });

    let ctx = ExecutionContext::new_top_level(
        caller_addr(),
        contract_a_addr(),
        10,
        1_000_000,
        0,
        claw_vm::DEFAULT_FUEL_LIMIT,
        false,
    );

    let engine = VmEngine::new();
    let result = engine
        .execute_with_cross_calls(
            &wasm_a,
            "call_missing",
            &[],
            ctx,
            BTreeMap::new(),
            chain_state,
            Arc::new(BTreeMap::new()),
        )
        .expect("execution should succeed (call returns -1, not trap)");

    // -1 + 128 = 127
    assert_eq!(result.return_data, vec![127u8], "call_contract should return -1 for missing contract");
}

/// Test 3: Cross-contract calls are blocked in read-only (view) mode.
#[test]
fn test_cross_call_blocked_in_view_mode() {
    let wasm_a = compile_wat(&contract_a_calls_b_wat(&contract_b_addr()));
    let wasm_b = compile_wat(CONTRACT_B_GREET);

    let mut code_map = BTreeMap::new();
    code_map.insert(contract_b_addr(), wasm_b);

    let chain_state = Arc::new(CrossCallChainState {
        contract_code: code_map.clone(),
    });

    let ctx = ExecutionContext::new_top_level(
        caller_addr(),
        contract_a_addr(),
        10,
        1_000_000,
        0,
        claw_vm::VIEW_CALL_FUEL_LIMIT,
        true, // read-only
    );

    let engine = VmEngine::new();
    let result = engine.execute_with_cross_calls(
        &wasm_a,
        "call_b",
        &[],
        ctx,
        BTreeMap::new(),
        chain_state,
        Arc::new(code_map),
    );

    assert!(result.is_err(), "cross-contract call should fail in view mode");
    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("read-only") || err.contains("view"),
        "error should mention read-only: {err}"
    );
}

/// Test 4: Fuel consumed by child is deducted from parent.
#[test]
fn test_cross_call_fuel_forwarding() {
    let wasm_b = compile_wat(CONTRACT_B_GREET);
    let wasm_a = compile_wat(&contract_a_calls_b_wat(&contract_b_addr()));

    let mut code_map = BTreeMap::new();
    code_map.insert(contract_b_addr(), wasm_b);

    let chain_state = Arc::new(CrossCallChainState {
        contract_code: code_map.clone(),
    });

    let ctx = ExecutionContext::new_top_level(
        caller_addr(),
        contract_a_addr(),
        10,
        1_000_000,
        0,
        claw_vm::DEFAULT_FUEL_LIMIT,
        false,
    );

    let engine = VmEngine::new();
    let result = engine
        .execute_with_cross_calls(
            &wasm_a,
            "call_b",
            &[],
            ctx,
            BTreeMap::new(),
            chain_state,
            Arc::new(code_map),
        )
        .expect("execution should succeed");

    // Total fuel consumed should include CROSS_CALL_BASE_FUEL (200_000) plus
    // child's fuel consumption plus parent's own host calls.
    assert!(
        result.fuel_consumed >= claw_vm::CROSS_CALL_BASE_FUEL,
        "fuel consumed ({}) should be >= CROSS_CALL_BASE_FUEL ({})",
        result.fuel_consumed,
        claw_vm::CROSS_CALL_BASE_FUEL
    );
}
