//! Integration tests for the `emit_event` host function and ContractEvent system.
//!
//! TDD flow:
//!   RED   — tests written before implementation
//!   GREEN — implementation added to types.rs / host.rs / engine.rs / constants.rs
//!   CHECK — edge cases: cap enforcement, oversized data, topic validation

use std::collections::BTreeMap;

use claw_vm::{ChainState, ExecutionContext, VmEngine};

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
    fn get_contract_code(&self, _contract: &[u8; 32]) -> Option<Vec<u8>> {
        None
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn make_context() -> ExecutionContext {
    ExecutionContext::new_top_level(
        [0u8; 32],
        [1u8; 32],
        10,
        1_000_000,
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

/// A contract that emits a single event with topic "transfer" and 8 bytes of data.
///
/// Memory layout:
///   offset 0:  "transfer" (8 bytes, ASCII)
///   offset 8:  data payload (8 bytes: 0x01 0x02 0x03 0x04 0x05 0x06 0x07 0x08)
const CONTRACT_EMIT_ONE_EVENT: &str = r#"
(module
  (import "env" "emit_event"      (func $emit_event    (param i32 i32 i32 i32)))
  (import "env" "storage_read"    (func $storage_read  (param i32 i32 i32) (result i32)))
  (import "env" "storage_write"   (func $storage_write (param i32 i32 i32 i32)))
  (import "env" "storage_has"     (func $storage_has   (param i32 i32) (result i32)))
  (import "env" "storage_delete"  (func $storage_delete (param i32 i32)))
  (import "env" "caller"          (func $caller        (param i32)))
  (import "env" "block_height"    (func $block_height  (result i64)))
  (import "env" "block_timestamp" (func $block_timestamp (result i64)))
  (import "env" "contract_address" (func $contract_address (param i32)))
  (import "env" "value_lo"        (func $value_lo      (result i64)))
  (import "env" "value_hi"        (func $value_hi      (result i64)))
  (import "env" "agent_get_score" (func $agent_get_score (param i32) (result i64)))
  (import "env" "agent_is_registered" (func $agent_is_registered (param i32) (result i32)))
  (import "env" "token_balance"   (func $token_balance (param i32) (result i64)))
  (import "env" "token_balance_hi" (func $token_balance_hi (param i32) (result i64)))
  (import "env" "token_transfer"  (func $token_transfer (param i32 i64 i64) (result i32)))
  (import "env" "log_msg"         (func $log_msg       (param i32 i32)))
  (import "env" "return_data"     (func $return_data   (param i32 i32)))
  (import "env" "abort"           (func $abort         (param i32 i32)))

  (memory (export "memory") 1)

  ;; offset 0:  "transfer" topic (8 bytes)
  ;; offset 8:  event data (8 bytes: sequential values)
  (data (i32.const 0) "transfer")
  (data (i32.const 8) "\01\02\03\04\05\06\07\08")

  (func (export "do_emit")
    ;; emit_event(topic_ptr=0, topic_len=8, data_ptr=8, data_len=8)
    (call $emit_event (i32.const 0) (i32.const 8) (i32.const 8) (i32.const 8))
  )
)
"#;

/// A contract that emits exactly MAX_EVENTS_PER_EXECUTION events (should succeed).
/// We use a loop that calls emit_event 50 times.
const CONTRACT_EMIT_EXACTLY_50: &str = r#"
(module
  (import "env" "emit_event"      (func $emit_event    (param i32 i32 i32 i32)))
  (import "env" "storage_read"    (func $storage_read  (param i32 i32 i32) (result i32)))
  (import "env" "storage_write"   (func $storage_write (param i32 i32 i32 i32)))
  (import "env" "storage_has"     (func $storage_has   (param i32 i32) (result i32)))
  (import "env" "storage_delete"  (func $storage_delete (param i32 i32)))
  (import "env" "caller"          (func $caller        (param i32)))
  (import "env" "block_height"    (func $block_height  (result i64)))
  (import "env" "block_timestamp" (func $block_timestamp (result i64)))
  (import "env" "contract_address" (func $contract_address (param i32)))
  (import "env" "value_lo"        (func $value_lo      (result i64)))
  (import "env" "value_hi"        (func $value_hi      (result i64)))
  (import "env" "agent_get_score" (func $agent_get_score (param i32) (result i64)))
  (import "env" "agent_is_registered" (func $agent_is_registered (param i32) (result i32)))
  (import "env" "token_balance"   (func $token_balance (param i32) (result i64)))
  (import "env" "token_balance_hi" (func $token_balance_hi (param i32) (result i64)))
  (import "env" "token_transfer"  (func $token_transfer (param i32 i64 i64) (result i32)))
  (import "env" "log_msg"         (func $log_msg       (param i32 i32)))
  (import "env" "return_data"     (func $return_data   (param i32 i32)))
  (import "env" "abort"           (func $abort         (param i32 i32)))

  (memory (export "memory") 1)

  ;; offset 0: "evt" topic (3 bytes)
  ;; offset 3: data (1 byte: 0x42)
  (data (i32.const 0) "evt")
  (data (i32.const 3) "\42")

  ;; Loop counter at offset 100
  (global $counter (mut i32) (i32.const 0))

  (func (export "emit_50")
    (local $i i32)
    (local.set $i (i32.const 0))
    (block $break
      (loop $loop
        (br_if $break (i32.ge_u (local.get $i) (i32.const 50)))
        (call $emit_event (i32.const 0) (i32.const 3) (i32.const 3) (i32.const 1))
        (local.set $i (i32.add (local.get $i) (i32.const 1)))
        (br $loop)
      )
    )
  )
)
"#;

/// A contract that tries to emit 51 events — the 51st must trap execution.
const CONTRACT_EMIT_51_TRAPS: &str = r#"
(module
  (import "env" "emit_event"      (func $emit_event    (param i32 i32 i32 i32)))
  (import "env" "storage_read"    (func $storage_read  (param i32 i32 i32) (result i32)))
  (import "env" "storage_write"   (func $storage_write (param i32 i32 i32 i32)))
  (import "env" "storage_has"     (func $storage_has   (param i32 i32) (result i32)))
  (import "env" "storage_delete"  (func $storage_delete (param i32 i32)))
  (import "env" "caller"          (func $caller        (param i32)))
  (import "env" "block_height"    (func $block_height  (result i64)))
  (import "env" "block_timestamp" (func $block_timestamp (result i64)))
  (import "env" "contract_address" (func $contract_address (param i32)))
  (import "env" "value_lo"        (func $value_lo      (result i64)))
  (import "env" "value_hi"        (func $value_hi      (result i64)))
  (import "env" "agent_get_score" (func $agent_get_score (param i32) (result i64)))
  (import "env" "agent_is_registered" (func $agent_is_registered (param i32) (result i32)))
  (import "env" "token_balance"   (func $token_balance (param i32) (result i64)))
  (import "env" "token_balance_hi" (func $token_balance_hi (param i32) (result i64)))
  (import "env" "token_transfer"  (func $token_transfer (param i32 i64 i64) (result i32)))
  (import "env" "log_msg"         (func $log_msg       (param i32 i32)))
  (import "env" "return_data"     (func $return_data   (param i32 i32)))
  (import "env" "abort"           (func $abort         (param i32 i32)))

  (memory (export "memory") 1)

  (data (i32.const 0) "evt")
  (data (i32.const 3) "\42")

  (func (export "emit_51")
    (local $i i32)
    (local.set $i (i32.const 0))
    (block $break
      (loop $loop
        (br_if $break (i32.ge_u (local.get $i) (i32.const 51)))
        (call $emit_event (i32.const 0) (i32.const 3) (i32.const 3) (i32.const 1))
        (local.set $i (i32.add (local.get $i) (i32.const 1)))
        (br $loop)
      )
    )
  )
)
"#;

/// A contract that emits an event with data exceeding MAX_EVENT_DATA_SIZE (4096 bytes).
/// The data pointer points to mem offset 100, length claimed is 4097.
/// This should trap because 4097 > 4096.
const CONTRACT_EMIT_OVERSIZED_DATA: &str = r#"
(module
  (import "env" "emit_event"      (func $emit_event    (param i32 i32 i32 i32)))
  (import "env" "storage_read"    (func $storage_read  (param i32 i32 i32) (result i32)))
  (import "env" "storage_write"   (func $storage_write (param i32 i32 i32 i32)))
  (import "env" "storage_has"     (func $storage_has   (param i32 i32) (result i32)))
  (import "env" "storage_delete"  (func $storage_delete (param i32 i32)))
  (import "env" "caller"          (func $caller        (param i32)))
  (import "env" "block_height"    (func $block_height  (result i64)))
  (import "env" "block_timestamp" (func $block_timestamp (result i64)))
  (import "env" "contract_address" (func $contract_address (param i32)))
  (import "env" "value_lo"        (func $value_lo      (result i64)))
  (import "env" "value_hi"        (func $value_hi      (result i64)))
  (import "env" "agent_get_score" (func $agent_get_score (param i32) (result i64)))
  (import "env" "agent_is_registered" (func $agent_is_registered (param i32) (result i32)))
  (import "env" "token_balance"   (func $token_balance (param i32) (result i64)))
  (import "env" "token_balance_hi" (func $token_balance_hi (param i32) (result i64)))
  (import "env" "token_transfer"  (func $token_transfer (param i32 i64 i64) (result i32)))
  (import "env" "log_msg"         (func $log_msg       (param i32 i32)))
  (import "env" "return_data"     (func $return_data   (param i32 i32)))
  (import "env" "abort"           (func $abort         (param i32 i32)))

  (memory (export "memory") 2)

  (data (i32.const 0) "bigdata")

  (func (export "emit_oversized")
    ;; topic: "bigdata" (7 bytes), data: ptr=100, len=4097 (exceeds 4096 limit)
    (call $emit_event (i32.const 0) (i32.const 7) (i32.const 100) (i32.const 4097))
  )
)
"#;

/// A contract that emits exactly MAX_EVENT_DATA_SIZE bytes of data (boundary: should succeed).
const CONTRACT_EMIT_EXACT_MAX_DATA: &str = r#"
(module
  (import "env" "emit_event"      (func $emit_event    (param i32 i32 i32 i32)))
  (import "env" "storage_read"    (func $storage_read  (param i32 i32 i32) (result i32)))
  (import "env" "storage_write"   (func $storage_write (param i32 i32 i32 i32)))
  (import "env" "storage_has"     (func $storage_has   (param i32 i32) (result i32)))
  (import "env" "storage_delete"  (func $storage_delete (param i32 i32)))
  (import "env" "caller"          (func $caller        (param i32)))
  (import "env" "block_height"    (func $block_height  (result i64)))
  (import "env" "block_timestamp" (func $block_timestamp (result i64)))
  (import "env" "contract_address" (func $contract_address (param i32)))
  (import "env" "value_lo"        (func $value_lo      (result i64)))
  (import "env" "value_hi"        (func $value_hi      (result i64)))
  (import "env" "agent_get_score" (func $agent_get_score (param i32) (result i64)))
  (import "env" "agent_is_registered" (func $agent_is_registered (param i32) (result i32)))
  (import "env" "token_balance"   (func $token_balance (param i32) (result i64)))
  (import "env" "token_balance_hi" (func $token_balance_hi (param i32) (result i64)))
  (import "env" "token_transfer"  (func $token_transfer (param i32 i64 i64) (result i32)))
  (import "env" "log_msg"         (func $log_msg       (param i32 i32)))
  (import "env" "return_data"     (func $return_data   (param i32 i32)))
  (import "env" "abort"           (func $abort         (param i32 i32)))

  (memory (export "memory") 2)

  (data (i32.const 0) "boundary")

  (func (export "emit_max_data")
    ;; topic: "boundary" (8 bytes), data: ptr=100, len=4096 (exactly at limit)
    (call $emit_event (i32.const 0) (i32.const 8) (i32.const 100) (i32.const 4096))
  )
)
"#;

/// A contract that emits an event with empty topic — should trap (invalid topic).
const CONTRACT_EMIT_EMPTY_TOPIC: &str = r#"
(module
  (import "env" "emit_event"      (func $emit_event    (param i32 i32 i32 i32)))
  (import "env" "storage_read"    (func $storage_read  (param i32 i32 i32) (result i32)))
  (import "env" "storage_write"   (func $storage_write (param i32 i32 i32 i32)))
  (import "env" "storage_has"     (func $storage_has   (param i32 i32) (result i32)))
  (import "env" "storage_delete"  (func $storage_delete (param i32 i32)))
  (import "env" "caller"          (func $caller        (param i32)))
  (import "env" "block_height"    (func $block_height  (result i64)))
  (import "env" "block_timestamp" (func $block_timestamp (result i64)))
  (import "env" "contract_address" (func $contract_address (param i32)))
  (import "env" "value_lo"        (func $value_lo      (result i64)))
  (import "env" "value_hi"        (func $value_hi      (result i64)))
  (import "env" "agent_get_score" (func $agent_get_score (param i32) (result i64)))
  (import "env" "agent_is_registered" (func $agent_is_registered (param i32) (result i32)))
  (import "env" "token_balance"   (func $token_balance (param i32) (result i64)))
  (import "env" "token_balance_hi" (func $token_balance_hi (param i32) (result i64)))
  (import "env" "token_transfer"  (func $token_transfer (param i32 i64 i64) (result i32)))
  (import "env" "log_msg"         (func $log_msg       (param i32 i32)))
  (import "env" "return_data"     (func $return_data   (param i32 i32)))
  (import "env" "abort"           (func $abort         (param i32 i32)))

  (memory (export "memory") 1)

  (func (export "emit_empty_topic")
    ;; topic_len = 0 → empty topic, must trap
    (call $emit_event (i32.const 0) (i32.const 0) (i32.const 0) (i32.const 0))
  )
)
"#;

/// A contract that emits zero-length data (empty payload) — should succeed.
const CONTRACT_EMIT_EMPTY_DATA: &str = r#"
(module
  (import "env" "emit_event"      (func $emit_event    (param i32 i32 i32 i32)))
  (import "env" "storage_read"    (func $storage_read  (param i32 i32 i32) (result i32)))
  (import "env" "storage_write"   (func $storage_write (param i32 i32 i32 i32)))
  (import "env" "storage_has"     (func $storage_has   (param i32 i32) (result i32)))
  (import "env" "storage_delete"  (func $storage_delete (param i32 i32)))
  (import "env" "caller"          (func $caller        (param i32)))
  (import "env" "block_height"    (func $block_height  (result i64)))
  (import "env" "block_timestamp" (func $block_timestamp (result i64)))
  (import "env" "contract_address" (func $contract_address (param i32)))
  (import "env" "value_lo"        (func $value_lo      (result i64)))
  (import "env" "value_hi"        (func $value_hi      (result i64)))
  (import "env" "agent_get_score" (func $agent_get_score (param i32) (result i64)))
  (import "env" "agent_is_registered" (func $agent_is_registered (param i32) (result i32)))
  (import "env" "token_balance"   (func $token_balance (param i32) (result i64)))
  (import "env" "token_balance_hi" (func $token_balance_hi (param i32) (result i64)))
  (import "env" "token_transfer"  (func $token_transfer (param i32 i64 i64) (result i32)))
  (import "env" "log_msg"         (func $log_msg       (param i32 i32)))
  (import "env" "return_data"     (func $return_data   (param i32 i32)))
  (import "env" "abort"           (func $abort         (param i32 i32)))

  (memory (export "memory") 1)

  (data (i32.const 0) "ping")

  (func (export "emit_empty_data")
    ;; topic: "ping" (4 bytes), data: ptr=0, len=0 (empty data is valid)
    (call $emit_event (i32.const 0) (i32.const 4) (i32.const 0) (i32.const 0))
  )
)
"#;

// ---------------------------------------------------------------------------
// Tests (RED phase — written before implementation)
// ---------------------------------------------------------------------------

/// Test 1: Contract emits a single event → event appears in ExecutionResult.events
#[test]
fn test_emit_event_appears_in_result() {
    let wasm = compile_wat(CONTRACT_EMIT_ONE_EVENT);
    let engine = VmEngine::new();
    let result = engine
        .execute(
            &wasm,
            "do_emit",
            &[],
            make_context(),
            BTreeMap::new(),
            &TestChainState,
        )
        .expect("execution must succeed");

    assert_eq!(result.events.len(), 1, "exactly one event must be emitted");

    let event = &result.events[0];
    assert_eq!(event.topic, "transfer", "topic must be 'transfer'");
    assert_eq!(
        event.data,
        vec![0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08],
        "event data must match the 8 bytes written"
    );
}

/// Test 2: Emitting exactly 50 events (MAX_EVENTS_PER_EXECUTION) succeeds.
#[test]
fn test_emit_exactly_50_events_succeeds() {
    let wasm = compile_wat(CONTRACT_EMIT_EXACTLY_50);
    let engine = VmEngine::new();
    let result = engine
        .execute(
            &wasm,
            "emit_50",
            &[],
            make_context(),
            BTreeMap::new(),
            &TestChainState,
        )
        .expect("emitting exactly 50 events must succeed");

    assert_eq!(
        result.events.len(),
        50,
        "all 50 events must be present in result"
    );

    // Verify topic and data on every event
    for (i, event) in result.events.iter().enumerate() {
        assert_eq!(event.topic, "evt", "event {i} topic must be 'evt'");
        assert_eq!(event.data, vec![0x42], "event {i} data must be [0x42]");
    }
}

/// Test 3: The 51st emit_event call traps execution (returns Err).
#[test]
fn test_emit_51st_event_traps() {
    let wasm = compile_wat(CONTRACT_EMIT_51_TRAPS);
    let engine = VmEngine::new();
    let result = engine.execute(
        &wasm,
        "emit_51",
        &[],
        make_context(),
        BTreeMap::new(),
        &TestChainState,
    );

    assert!(
        result.is_err(),
        "emitting 51 events must trap: got {:?}",
        result
    );
    // The error message must mention the event cap
    let err_str = result.unwrap_err().to_string();
    assert!(
        err_str.contains("event") || err_str.contains("limit") || err_str.contains("max"),
        "error message must reference event cap, got: {err_str}"
    );
}

/// Test 4: Event data exceeding MAX_EVENT_DATA_SIZE (4097 bytes) traps execution.
#[test]
fn test_emit_oversized_data_traps() {
    let wasm = compile_wat(CONTRACT_EMIT_OVERSIZED_DATA);
    let engine = VmEngine::new();
    let result = engine.execute(
        &wasm,
        "emit_oversized",
        &[],
        make_context(),
        BTreeMap::new(),
        &TestChainState,
    );

    assert!(
        result.is_err(),
        "emitting oversized data must trap: got {:?}",
        result
    );
}

/// Test 5: Boundary — exactly MAX_EVENT_DATA_SIZE (4096 bytes) of data succeeds.
#[test]
fn test_emit_exact_max_data_size_succeeds() {
    let wasm = compile_wat(CONTRACT_EMIT_EXACT_MAX_DATA);
    let engine = VmEngine::new();
    let result = engine
        .execute(
            &wasm,
            "emit_max_data",
            &[],
            make_context(),
            BTreeMap::new(),
            &TestChainState,
        )
        .expect("emitting exactly 4096 bytes of data must succeed");

    assert_eq!(result.events.len(), 1, "one event must be emitted");
    assert_eq!(
        result.events[0].data.len(),
        4096,
        "data length must be 4096 bytes"
    );
    assert_eq!(result.events[0].topic, "boundary");
}

/// Test 6: Empty topic traps execution (empty string is not a valid topic).
#[test]
fn test_emit_empty_topic_traps() {
    let wasm = compile_wat(CONTRACT_EMIT_EMPTY_TOPIC);
    let engine = VmEngine::new();
    let result = engine.execute(
        &wasm,
        "emit_empty_topic",
        &[],
        make_context(),
        BTreeMap::new(),
        &TestChainState,
    );

    assert!(
        result.is_err(),
        "emitting with empty topic must trap: got {:?}",
        result
    );
}

/// Test 7: Empty data payload (len=0) is valid — should succeed.
#[test]
fn test_emit_empty_data_succeeds() {
    let wasm = compile_wat(CONTRACT_EMIT_EMPTY_DATA);
    let engine = VmEngine::new();
    let result = engine
        .execute(
            &wasm,
            "emit_empty_data",
            &[],
            make_context(),
            BTreeMap::new(),
            &TestChainState,
        )
        .expect("emitting with empty data must succeed");

    assert_eq!(result.events.len(), 1);
    assert_eq!(result.events[0].topic, "ping");
    assert!(result.events[0].data.is_empty(), "data must be empty");
}

/// Test 8: Events do not appear in result when execution fails.
/// (51-event trap: even if 50 were buffered, they must not leak into any partial result.)
#[test]
fn test_events_not_leaked_on_trap() {
    let wasm = compile_wat(CONTRACT_EMIT_51_TRAPS);
    let engine = VmEngine::new();
    let result = engine.execute(
        &wasm,
        "emit_51",
        &[],
        make_context(),
        BTreeMap::new(),
        &TestChainState,
    );

    // The result must be Err — no partial success with some events.
    assert!(result.is_err(), "must return Err on trap");
}

/// Test 9: Multiple events preserve insertion order.
#[test]
fn test_events_preserve_order() {
    // Contract that emits 3 events with different topics in order.
    const CONTRACT_THREE_EVENTS_ORDERED: &str = r#"
(module
  (import "env" "emit_event"      (func $emit_event    (param i32 i32 i32 i32)))
  (import "env" "storage_read"    (func $storage_read  (param i32 i32 i32) (result i32)))
  (import "env" "storage_write"   (func $storage_write (param i32 i32 i32 i32)))
  (import "env" "storage_has"     (func $storage_has   (param i32 i32) (result i32)))
  (import "env" "storage_delete"  (func $storage_delete (param i32 i32)))
  (import "env" "caller"          (func $caller        (param i32)))
  (import "env" "block_height"    (func $block_height  (result i64)))
  (import "env" "block_timestamp" (func $block_timestamp (result i64)))
  (import "env" "contract_address" (func $contract_address (param i32)))
  (import "env" "value_lo"        (func $value_lo      (result i64)))
  (import "env" "value_hi"        (func $value_hi      (result i64)))
  (import "env" "agent_get_score" (func $agent_get_score (param i32) (result i64)))
  (import "env" "agent_is_registered" (func $agent_is_registered (param i32) (result i32)))
  (import "env" "token_balance"   (func $token_balance (param i32) (result i64)))
  (import "env" "token_balance_hi" (func $token_balance_hi (param i32) (result i64)))
  (import "env" "token_transfer"  (func $token_transfer (param i32 i64 i64) (result i32)))
  (import "env" "log_msg"         (func $log_msg       (param i32 i32)))
  (import "env" "return_data"     (func $return_data   (param i32 i32)))
  (import "env" "abort"           (func $abort         (param i32 i32)))

  (memory (export "memory") 1)

  ;; offset 0:  "first"  (5 bytes)
  ;; offset 5:  "second" (6 bytes)
  ;; offset 11: "third"  (5 bytes)
  ;; offset 16: data byte: 0x01
  ;; offset 17: data byte: 0x02
  ;; offset 18: data byte: 0x03
  (data (i32.const 0)  "first")
  (data (i32.const 5)  "second")
  (data (i32.const 11) "third")
  (data (i32.const 16) "\01")
  (data (i32.const 17) "\02")
  (data (i32.const 18) "\03")

  (func (export "emit_three")
    (call $emit_event (i32.const 0)  (i32.const 5) (i32.const 16) (i32.const 1))
    (call $emit_event (i32.const 5)  (i32.const 6) (i32.const 17) (i32.const 1))
    (call $emit_event (i32.const 11) (i32.const 5) (i32.const 18) (i32.const 1))
  )
)
"#;

    let wasm = compile_wat(CONTRACT_THREE_EVENTS_ORDERED);
    let engine = VmEngine::new();
    let result = engine
        .execute(
            &wasm,
            "emit_three",
            &[],
            make_context(),
            BTreeMap::new(),
            &TestChainState,
        )
        .expect("three events must succeed");

    assert_eq!(result.events.len(), 3);
    assert_eq!(result.events[0].topic, "first");
    assert_eq!(result.events[0].data, vec![0x01]);
    assert_eq!(result.events[1].topic, "second");
    assert_eq!(result.events[1].data, vec![0x02]);
    assert_eq!(result.events[2].topic, "third");
    assert_eq!(result.events[2].data, vec![0x03]);
}

/// Test 10: No events emitted when contract doesn't call emit_event.
#[test]
fn test_no_events_when_not_emitted() {
    // Re-use the lo-only balance contract from the balance test which never emits
    const CONTRACT_NOOP: &str = r#"
(module
  (import "env" "emit_event"      (func $emit_event    (param i32 i32 i32 i32)))
  (import "env" "storage_read"    (func $storage_read  (param i32 i32 i32) (result i32)))
  (import "env" "storage_write"   (func $storage_write (param i32 i32 i32 i32)))
  (import "env" "storage_has"     (func $storage_has   (param i32 i32) (result i32)))
  (import "env" "storage_delete"  (func $storage_delete (param i32 i32)))
  (import "env" "caller"          (func $caller        (param i32)))
  (import "env" "block_height"    (func $block_height  (result i64)))
  (import "env" "block_timestamp" (func $block_timestamp (result i64)))
  (import "env" "contract_address" (func $contract_address (param i32)))
  (import "env" "value_lo"        (func $value_lo      (result i64)))
  (import "env" "value_hi"        (func $value_hi      (result i64)))
  (import "env" "agent_get_score" (func $agent_get_score (param i32) (result i64)))
  (import "env" "agent_is_registered" (func $agent_is_registered (param i32) (result i32)))
  (import "env" "token_balance"   (func $token_balance (param i32) (result i64)))
  (import "env" "token_balance_hi" (func $token_balance_hi (param i32) (result i64)))
  (import "env" "token_transfer"  (func $token_transfer (param i32 i64 i64) (result i32)))
  (import "env" "log_msg"         (func $log_msg       (param i32 i32)))
  (import "env" "return_data"     (func $return_data   (param i32 i32)))
  (import "env" "abort"           (func $abort         (param i32 i32)))

  (memory (export "memory") 1)

  (func (export "noop")
    ;; does nothing
  )
)
"#;

    let wasm = compile_wat(CONTRACT_NOOP);
    let engine = VmEngine::new();
    let result = engine
        .execute(
            &wasm,
            "noop",
            &[],
            make_context(),
            BTreeMap::new(),
            &TestChainState,
        )
        .expect("noop contract must succeed");

    assert!(
        result.events.is_empty(),
        "no events should be present when none were emitted"
    );
}
