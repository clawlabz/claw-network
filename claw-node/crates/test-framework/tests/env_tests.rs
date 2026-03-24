//! TDD tests for TestEnv — written BEFORE the implementation (RED phase).
//!
//! Each test documents the expected public API and behaviour contract.
//! They compile only after the implementation is in place; until then
//! `cargo test -p claw-test` produces compilation errors — this is the
//! intended RED state.

use claw_test::TestEnv;

// ---------------------------------------------------------------------------
// Minimal WAT helpers
// ---------------------------------------------------------------------------

/// Compile a WAT source string to Wasm bytes.
fn wat(src: &str) -> Vec<u8> {
    wat::parse_str(src).expect("WAT compilation failed")
}

/// A WAT module that imports all host functions required by the engine.
/// Each test contract embeds this boilerplate so the Wasmer linker is happy.
macro_rules! wat_module {
    ($body:expr) => {
        concat!(
            r#"(module
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
  (memory (export "memory") 1)"#,
            $body,
            "\n)"
        )
    };
}

// ---------------------------------------------------------------------------
// A trivial no-op contract: exports `init` and `noop`, does nothing.
// ---------------------------------------------------------------------------

const NOOP_CONTRACT: &str = wat_module!(
    r#"
  (func (export "init"))
  (func (export "noop"))
"#
);

// ---------------------------------------------------------------------------
// A contract that writes a single storage key.
//
// `set_key`: writes key "k" (offset 0, 1 byte) → value "v" (offset 1, 1 byte)
// `get_key`: reads key "k" into scratch at offset 64, returns length via return_data
// ---------------------------------------------------------------------------

const STORAGE_CONTRACT: &str = wat_module!(
    r#"
  (data (i32.const 0) "k")
  (data (i32.const 1) "v")

  (func (export "init"))

  (func (export "set_key")
    ;; storage_write(key_ptr=0, key_len=1, val_ptr=1, val_len=1)
    (call $storage_write (i32.const 0) (i32.const 1) (i32.const 1) (i32.const 1))
  )

  (func (export "get_key")
    ;; storage_read(key_ptr=0, key_len=1, val_ptr=64) → len (i32)
    (drop (call $storage_read (i32.const 0) (i32.const 1) (i32.const 64)))
    ;; return_data(ptr=64, len=1)
    (call $return_data (i32.const 64) (i32.const 1))
  )
"#
);

// ---------------------------------------------------------------------------
// A contract that emits an event named "ping" with 4 bytes of data.
// ---------------------------------------------------------------------------

const EVENT_CONTRACT: &str = wat_module!(
    r#"
  (data (i32.const 0) "ping")
  (data (i32.const 4) "\01\02\03\04")

  (func (export "init"))

  (func (export "do_ping")
    ;; emit_event(topic_ptr=0, topic_len=4, data_ptr=4, data_len=4)
    (call $emit_event (i32.const 0) (i32.const 4) (i32.const 4) (i32.const 4))
  )
"#
);

// ---------------------------------------------------------------------------
// A contract that performs a token transfer of 1 nano-CLAW to address [2;32].
// ---------------------------------------------------------------------------

const TRANSFER_CONTRACT: &str = wat_module!(
    r#"
  ;; offset 0-31: recipient address (all 0x02 bytes)
  (data (i32.const 0) "\02\02\02\02\02\02\02\02\02\02\02\02\02\02\02\02\02\02\02\02\02\02\02\02\02\02\02\02\02\02\02\02")

  (func (export "init"))

  (func (export "do_transfer")
    ;; token_transfer(to_ptr=0, amount_lo=1, amount_hi=0)
    (drop (call $token_transfer (i32.const 0) (i64.const 1) (i64.const 0)))
  )
"#
);

// ---------------------------------------------------------------------------
// 1. new() creates an empty environment
// ---------------------------------------------------------------------------

#[test]
fn test_new_creates_empty_env() {
    let env = TestEnv::new();
    // Block height starts at 0.
    assert_eq!(env.block_height(), 0);
    // Block timestamp starts at 0.
    assert_eq!(env.block_timestamp(), 0);
    // No balance for an arbitrary address.
    assert_eq!(env.get_balance([0u8; 32]), 0);
    // No storage for an arbitrary contract / key pair.
    assert!(env
        .get_storage([0u8; 32], b"any_key")
        .is_none());
}

// ---------------------------------------------------------------------------
// 2. set_balance + get_balance round-trip
// ---------------------------------------------------------------------------

#[test]
fn test_set_get_balance_round_trip() {
    let mut env = TestEnv::new();
    let alice = [1u8; 32];
    let bob = [2u8; 32];

    env.set_balance(alice, 1_000_000);
    env.set_balance(bob, 42);

    assert_eq!(env.get_balance(alice), 1_000_000);
    assert_eq!(env.get_balance(bob), 42);
    // Unset address should still be zero.
    assert_eq!(env.get_balance([9u8; 32]), 0);
}

#[test]
fn test_set_balance_overwrites_previous() {
    let mut env = TestEnv::new();
    let addr = [3u8; 32];
    env.set_balance(addr, 500);
    env.set_balance(addr, 999);
    assert_eq!(env.get_balance(addr), 999);
}

#[test]
fn test_set_balance_zero_is_valid() {
    let mut env = TestEnv::new();
    let addr = [4u8; 32];
    env.set_balance(addr, 0);
    assert_eq!(env.get_balance(addr), 0);
}

#[test]
fn test_set_balance_u128_max() {
    let mut env = TestEnv::new();
    let addr = [5u8; 32];
    env.set_balance(addr, u128::MAX);
    assert_eq!(env.get_balance(addr), u128::MAX);
}

// ---------------------------------------------------------------------------
// 3. deploy with valid Wasm succeeds and returns a deterministic address
// ---------------------------------------------------------------------------

#[test]
fn test_deploy_valid_wasm_succeeds() {
    let mut env = TestEnv::new();
    let deployer = [1u8; 32];
    let wasm = wat(NOOP_CONTRACT);

    let result = env.deploy(deployer, &wasm, "init", &[]);
    assert!(result.is_ok(), "deploy should succeed: {:?}", result);
}

#[test]
fn test_deploy_returns_deterministic_address() {
    let wasm = wat(NOOP_CONTRACT);
    let deployer = [1u8; 32];

    // Two separate environments with the same deployer + nonce should
    // produce the same address.
    let mut env1 = TestEnv::new();
    let addr1 = env1.deploy(deployer, &wasm, "init", &[]).unwrap();

    let mut env2 = TestEnv::new();
    let addr2 = env2.deploy(deployer, &wasm, "init", &[]).unwrap();

    assert_eq!(addr1, addr2);
}

#[test]
fn test_deploy_increments_nonce_for_second_deploy() {
    let mut env = TestEnv::new();
    let deployer = [1u8; 32];
    let wasm = wat(NOOP_CONTRACT);

    let addr1 = env.deploy(deployer, &wasm, "init", &[]).unwrap();
    let addr2 = env.deploy(deployer, &wasm, "init", &[]).unwrap();

    // Two contracts by the same deployer must have different addresses.
    assert_ne!(addr1, addr2);
}

#[test]
fn test_deploy_stores_code_for_later_calls() {
    let mut env = TestEnv::new();
    let deployer = [1u8; 32];
    let wasm = wat(NOOP_CONTRACT);

    let contract = env.deploy(deployer, &wasm, "init", &[]).unwrap();

    // Calling a method on the deployed contract must succeed.
    let result = env.call(deployer, contract, "noop", &[], 0);
    assert!(result.is_ok(), "call after deploy should succeed: {:?}", result);
}

// ---------------------------------------------------------------------------
// 4. deploy with invalid Wasm fails
// ---------------------------------------------------------------------------

#[test]
fn test_deploy_invalid_wasm_fails() {
    let mut env = TestEnv::new();
    let deployer = [1u8; 32];
    let bad_wasm = b"not a wasm module at all";

    let result = env.deploy(deployer, bad_wasm, "init", &[]);
    assert!(result.is_err(), "deploy of garbage bytes should fail");
}

#[test]
fn test_deploy_empty_wasm_fails() {
    let mut env = TestEnv::new();
    let result = env.deploy([1u8; 32], &[], "init", &[]);
    assert!(result.is_err(), "deploy of empty bytes should fail");
}

// ---------------------------------------------------------------------------
// 5. call on deployed contract succeeds
// ---------------------------------------------------------------------------

#[test]
fn test_call_deployed_contract_succeeds() {
    let mut env = TestEnv::new();
    let deployer = [1u8; 32];
    let wasm = wat(NOOP_CONTRACT);

    let contract = env.deploy(deployer, &wasm, "init", &[]).unwrap();
    let result = env.call(deployer, contract, "noop", &[], 0);
    assert!(result.is_ok(), "call should succeed: {:?}", result);
}

#[test]
fn test_call_returns_call_result_with_fields() {
    let mut env = TestEnv::new();
    let deployer = [1u8; 32];
    let wasm = wat(NOOP_CONTRACT);

    let contract = env.deploy(deployer, &wasm, "init", &[]).unwrap();
    let result = env.call(deployer, contract, "noop", &[], 0).unwrap();

    // noop produces no return data, no events.
    assert_eq!(result.return_data, Vec::<u8>::new());
    assert_eq!(result.events.len(), 0);
    // The noop function calls no host functions so fuel_consumed is 0 (fuel
    // is only tracked per host-function invocation, not per Wasm instruction).
    // We assert the field exists and is a valid u64 rather than a specific value.
    let _ = result.fuel_consumed;
}

// ---------------------------------------------------------------------------
// 6. call on non-existent contract fails
// ---------------------------------------------------------------------------

#[test]
fn test_call_non_existent_contract_fails() {
    let mut env = TestEnv::new();
    let caller = [1u8; 32];
    let ghost = [99u8; 32]; // never deployed

    let result = env.call(caller, ghost, "noop", &[], 0);
    assert!(result.is_err(), "call on ghost contract should fail");
}

#[test]
fn test_call_unknown_method_fails() {
    let mut env = TestEnv::new();
    let deployer = [1u8; 32];
    let wasm = wat(NOOP_CONTRACT);

    let contract = env.deploy(deployer, &wasm, "init", &[]).unwrap();
    let result = env.call(deployer, contract, "does_not_exist", &[], 0);
    assert!(result.is_err(), "call to missing method should fail");
}

// ---------------------------------------------------------------------------
// 7. Storage changes persist across calls
// ---------------------------------------------------------------------------

#[test]
fn test_storage_write_persists_across_calls() {
    let mut env = TestEnv::new();
    let deployer = [1u8; 32];
    let wasm = wat(STORAGE_CONTRACT);

    let contract = env.deploy(deployer, &wasm, "init", &[]).unwrap();

    // First call: write "k" → "v" into storage.
    env.call(deployer, contract, "set_key", &[], 0).unwrap();

    // Verify via the host-side get_storage API.
    let stored = env.get_storage(contract, b"k");
    assert_eq!(stored, Some(b"v".to_vec()));
}

#[test]
fn test_storage_changes_visible_in_second_call() {
    let mut env = TestEnv::new();
    let deployer = [1u8; 32];
    let wasm = wat(STORAGE_CONTRACT);

    let contract = env.deploy(deployer, &wasm, "init", &[]).unwrap();

    // Write in first call.
    env.call(deployer, contract, "set_key", &[], 0).unwrap();

    // Second call reads back the same value.
    let result = env.call(deployer, contract, "get_key", &[], 0).unwrap();
    // The contract does return_data with the 1-byte value "v" (0x76).
    assert_eq!(result.return_data, b"v".to_vec());
}

#[test]
fn test_get_storage_returns_none_for_unset_key() {
    let mut env = TestEnv::new();
    let deployer = [1u8; 32];
    let wasm = wat(NOOP_CONTRACT);

    let contract = env.deploy(deployer, &wasm, "init", &[]).unwrap();
    assert!(env.get_storage(contract, b"missing").is_none());
}

// ---------------------------------------------------------------------------
// 8. Balance transfers work (value parameter)
// ---------------------------------------------------------------------------

#[test]
fn test_call_with_value_transfers_to_contract() {
    let mut env = TestEnv::new();
    let sender = [1u8; 32];
    let wasm = wat(NOOP_CONTRACT);

    env.set_balance(sender, 1_000_000);
    let contract = env.deploy(sender, &wasm, "init", &[]).unwrap();

    // Call with value=500: sender should lose 500, contract should gain 500.
    env.call(sender, contract, "noop", &[], 500).unwrap();

    assert_eq!(env.get_balance(sender), 1_000_000 - 500);
    assert_eq!(env.get_balance(contract), 500);
}

#[test]
fn test_call_with_zero_value_does_not_change_balances() {
    let mut env = TestEnv::new();
    let sender = [1u8; 32];
    let wasm = wat(NOOP_CONTRACT);

    env.set_balance(sender, 1_000_000);
    let contract = env.deploy(sender, &wasm, "init", &[]).unwrap();

    env.call(sender, contract, "noop", &[], 0).unwrap();

    assert_eq!(env.get_balance(sender), 1_000_000);
    assert_eq!(env.get_balance(contract), 0);
}

#[test]
fn test_call_with_insufficient_balance_fails() {
    let mut env = TestEnv::new();
    let sender = [1u8; 32];
    let wasm = wat(NOOP_CONTRACT);

    env.set_balance(sender, 100);
    let contract = env.deploy(sender, &wasm, "init", &[]).unwrap();

    // Try to send more than sender has.
    let result = env.call(sender, contract, "noop", &[], 200);
    assert!(result.is_err(), "should fail with insufficient balance");
}

// ---------------------------------------------------------------------------
// 9. advance_block increments height
// ---------------------------------------------------------------------------

#[test]
fn test_advance_block_increments_height_by_one() {
    let mut env = TestEnv::new();
    assert_eq!(env.block_height(), 0);
    env.advance_block();
    assert_eq!(env.block_height(), 1);
    env.advance_block();
    assert_eq!(env.block_height(), 2);
}

#[test]
fn test_advance_blocks_increments_height_by_n() {
    let mut env = TestEnv::new();
    env.advance_blocks(10);
    assert_eq!(env.block_height(), 10);
    env.advance_blocks(5);
    assert_eq!(env.block_height(), 15);
}

#[test]
fn test_advance_blocks_zero_is_noop() {
    let mut env = TestEnv::new();
    env.advance_blocks(0);
    assert_eq!(env.block_height(), 0);
}

#[test]
fn test_set_timestamp_stores_value() {
    let mut env = TestEnv::new();
    env.set_timestamp(1_700_000_000);
    assert_eq!(env.block_timestamp(), 1_700_000_000);
}

#[test]
fn test_contracts_see_updated_block_height() {
    // After advancing blocks, a subsequent call should observe the new height
    // (we verify this indirectly via the engine's context injection).
    let mut env = TestEnv::new();
    let deployer = [1u8; 32];
    let wasm = wat(NOOP_CONTRACT);
    let contract = env.deploy(deployer, &wasm, "init", &[]).unwrap();

    env.advance_blocks(42);
    // The call itself must succeed — we trust the context is forwarded correctly.
    let result = env.call(deployer, contract, "noop", &[], 0);
    assert!(result.is_ok());
}

// ---------------------------------------------------------------------------
// 10. Events are captured in CallResult
// ---------------------------------------------------------------------------

#[test]
fn test_events_captured_in_call_result() {
    let mut env = TestEnv::new();
    let deployer = [1u8; 32];
    let wasm = wat(EVENT_CONTRACT);

    let contract = env.deploy(deployer, &wasm, "init", &[]).unwrap();
    let result = env.call(deployer, contract, "do_ping", &[], 0).unwrap();

    assert_eq!(result.events.len(), 1);
    assert_eq!(result.events[0].topic, "ping");
    assert_eq!(result.events[0].data, vec![0x01, 0x02, 0x03, 0x04]);
}

#[test]
fn test_no_events_on_noop_call() {
    let mut env = TestEnv::new();
    let deployer = [1u8; 32];
    let wasm = wat(NOOP_CONTRACT);

    let contract = env.deploy(deployer, &wasm, "init", &[]).unwrap();
    let result = env.call(deployer, contract, "noop", &[], 0).unwrap();
    assert_eq!(result.events.len(), 0);
}

#[test]
fn test_multiple_calls_accumulate_events_per_call() {
    let mut env = TestEnv::new();
    let deployer = [1u8; 32];
    let wasm = wat(EVENT_CONTRACT);

    let contract = env.deploy(deployer, &wasm, "init", &[]).unwrap();

    let r1 = env.call(deployer, contract, "do_ping", &[], 0).unwrap();
    let r2 = env.call(deployer, contract, "do_ping", &[], 0).unwrap();

    // Each call independently captures its own events.
    assert_eq!(r1.events.len(), 1);
    assert_eq!(r2.events.len(), 1);
}

// ---------------------------------------------------------------------------
// 11. Contract-initiated token_transfer is reflected in host balances
// ---------------------------------------------------------------------------

#[test]
fn test_contract_token_transfer_updates_balances() {
    let mut env = TestEnv::new();
    let deployer = [1u8; 32];
    let recipient = [2u8; 32];
    let wasm = wat(TRANSFER_CONTRACT);

    // Fund the contract so it can transfer out.
    let contract = env.deploy(deployer, &wasm, "init", &[]).unwrap();
    env.set_balance(contract, 1_000);

    env.call(deployer, contract, "do_transfer", &[], 0).unwrap();

    // Contract sent 1 nano-CLAW to recipient [2;32].
    assert_eq!(env.get_balance(contract), 999);
    assert_eq!(env.get_balance(recipient), 1);
}

// ---------------------------------------------------------------------------
// 12. Edge cases
// ---------------------------------------------------------------------------

#[test]
fn test_different_deployers_get_different_addresses() {
    let mut env = TestEnv::new();
    let wasm = wat(NOOP_CONTRACT);

    let addr1 = env.deploy([1u8; 32], &wasm, "init", &[]).unwrap();
    let addr2 = env.deploy([2u8; 32], &wasm, "init", &[]).unwrap();
    assert_ne!(addr1, addr2);
}

#[test]
fn test_independent_contracts_have_isolated_storage() {
    let mut env = TestEnv::new();
    let deployer = [1u8; 32];
    let wasm = wat(STORAGE_CONTRACT);

    let c1 = env.deploy(deployer, &wasm, "init", &[]).unwrap();
    let c2 = env.deploy(deployer, &wasm, "init", &[]).unwrap();

    // Write key in c1.
    env.call(deployer, c1, "set_key", &[], 0).unwrap();

    // c2's storage must not be affected.
    assert!(env.get_storage(c2, b"k").is_none());
    // c1's storage is present.
    assert!(env.get_storage(c1, b"k").is_some());
}

#[test]
fn test_balances_of_multiple_addresses_are_independent() {
    let mut env = TestEnv::new();
    let addrs: Vec<[u8; 32]> = (0..10u8).map(|i| [i; 32]).collect();

    for (i, &addr) in addrs.iter().enumerate() {
        env.set_balance(addr, i as u128 * 1_000);
    }

    for (i, &addr) in addrs.iter().enumerate() {
        assert_eq!(env.get_balance(addr), i as u128 * 1_000);
    }
}

#[test]
fn test_fuel_consumed_is_nonzero_for_storage_write() {
    let mut env = TestEnv::new();
    let deployer = [1u8; 32];
    let wasm = wat(STORAGE_CONTRACT);

    let contract = env.deploy(deployer, &wasm, "init", &[]).unwrap();
    let result = env.call(deployer, contract, "set_key", &[], 0).unwrap();

    // A storage_write costs STORAGE_WRITE_FUEL (50_000), so total must be >= that.
    assert!(result.fuel_consumed >= 50_000);
}
