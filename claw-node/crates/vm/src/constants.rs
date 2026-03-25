pub const MAX_CONTRACT_CODE_SIZE: usize = 512 * 1024; // 512 KB
pub const DEFAULT_FUEL_LIMIT: u64 = 10_000_000; // 10M fuel
/// Fuel limit for read-only view calls — half of the normal limit.
pub const VIEW_CALL_FUEL_LIMIT: u64 = 5_000_000;
pub const STORAGE_READ_FUEL: u64 = 10_000;
pub const STORAGE_WRITE_FUEL: u64 = 50_000;
pub const STORAGE_DELETE_FUEL: u64 = 10_000;
pub const HOST_CALL_BASE_FUEL: u64 = 5_000;
pub const TOKEN_TRANSFER_FUEL: u64 = 100_000;
pub const AGENT_QUERY_FUEL: u64 = 10_000;

/// Fuel cost per `emit_event` host call.
pub const EVENT_EMIT_FUEL: u64 = 20_000;
/// Maximum number of events a single contract execution may emit.
pub const MAX_EVENTS_PER_EXECUTION: usize = 50;
/// Maximum byte length of the `data` payload in a single event.
pub const MAX_EVENT_DATA_SIZE: usize = 4096;

// ---------------------------------------------------------------------------
// Cross-contract call constants
// ---------------------------------------------------------------------------

/// Maximum nested call depth (A calls B calls C calls D = depth 4).
pub const MAX_CALL_DEPTH: u32 = 4;
/// Base fuel cost charged for each cross-contract call before forwarding
/// remaining fuel to the callee.
pub const CROSS_CALL_BASE_FUEL: u64 = 200_000;
/// Maximum byte length of return data from a cross-contract call (16 KB).
pub const MAX_CROSS_CALL_RETURN_SIZE: usize = 16 * 1024;
