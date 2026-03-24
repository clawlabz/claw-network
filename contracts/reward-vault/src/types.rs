//! Data types for the Reward Vault contract.
//!
//! All types are borsh-serializable so they can be stored in and retrieved
//! from contract storage without conversion overhead.

// On wasm32 (no_std) we pull String/Vec from `alloc`.
// On native targets std provides them.
extern crate alloc;
use alloc::{format, string::String, vec::Vec};

use borsh::{BorshDeserialize, BorshSerialize};

// ---------------------------------------------------------------------------
// Argument structs for each entry point
// ---------------------------------------------------------------------------

/// Arguments for the `init` entry point.
#[derive(BorshDeserialize, BorshSerialize, Debug, Clone)]
pub struct InitArgs {
    /// Contract owner address (32 bytes).
    pub owner: [u8; 32],
    /// Maximum CLAW (in nano-CLAW) claimable per address per UTC day.
    pub daily_cap: u128,
    /// Minimum game count required before a claim is valid.
    /// Currently stored but enforced off-chain by the platform caller.
    pub min_games: u64,
    /// Initial list of authorized platform addresses.
    pub platforms: Vec<[u8; 32]>,
}

/// Arguments for the `claim_reward` entry point.
#[derive(BorshDeserialize, BorshSerialize, Debug, Clone)]
pub struct ClaimRewardArgs {
    /// Recipient's address.
    pub recipient: [u8; 32],
    /// Amount to transfer (nano-CLAW).
    pub amount: u128,
    /// Monotonic nonce for replay protection; must equal the stored value.
    pub nonce: u64,
}

/// Arguments for the `fund` entry point (empty — value is passed via `get_value()`).
#[derive(BorshDeserialize, BorshSerialize, Debug, Clone)]
pub struct FundArgs {}

/// Arguments for the `set_daily_cap` entry point.
#[derive(BorshDeserialize, BorshSerialize, Debug, Clone)]
pub struct SetDailyCapArgs {
    pub new_cap: u128,
}

/// Arguments for the `add_platform` / `remove_platform` entry points.
#[derive(BorshDeserialize, BorshSerialize, Debug, Clone)]
pub struct PlatformArgs {
    pub addr: [u8; 32],
}

/// Arguments for the `withdraw` entry point.
#[derive(BorshDeserialize, BorshSerialize, Debug, Clone)]
pub struct WithdrawArgs {
    pub amount: u128,
}

/// Arguments for the `get_daily_claimed` view entry point.
#[derive(BorshDeserialize, BorshSerialize, Debug, Clone)]
pub struct GetDailyClaimedArgs {
    pub addr: [u8; 32],
}

/// Arguments for the `cleanup_claims` maintenance entry point.
#[derive(BorshDeserialize, BorshSerialize, Debug, Clone)]
pub struct CleanupClaimsArgs {
    /// Delete all `claimed:{hex}:{day}` records where `day < before_day`.
    pub before_day: u32,
    /// Addresses to clean up (bounded list to stay within gas limits).
    pub addrs: Vec<[u8; 32]>,
}

// ---------------------------------------------------------------------------
// Storage key helpers
// ---------------------------------------------------------------------------

/// Convert a 32-byte address to a lower-case hex string (64 chars).
pub fn addr_to_hex(addr: &[u8; 32]) -> String {
    addr.iter().map(|b| format!("{:02x}", b)).collect()
}

/// Build the storage key for a platform authorisation record.
///
/// Format: `platform:{hex_address}` (ASCII bytes)
pub fn platform_key(addr: &[u8; 32]) -> Vec<u8> {
    format!("platform:{}", addr_to_hex(addr)).into_bytes()
}

/// Build the storage key for a daily claimed amount.
///
/// Format: `claimed:{hex_address}:{day_number}` (ASCII bytes)
pub fn claimed_key(addr: &[u8; 32], day: u64) -> Vec<u8> {
    format!("claimed:{}:{}", addr_to_hex(addr), day).into_bytes()
}

/// Build the storage key for a per-address nonce.
///
/// Format: `nonce:{hex_address}` (ASCII bytes)
pub fn nonce_key(addr: &[u8; 32]) -> Vec<u8> {
    format!("nonce:{}", addr_to_hex(addr)).into_bytes()
}

/// Derive the UTC day number from a Unix timestamp (seconds).
///
/// `day = timestamp / 86_400`
pub fn day_from_timestamp(ts: u64) -> u64 {
    ts / 86_400
}

// ---------------------------------------------------------------------------
// Well-known storage keys (constants)
// ---------------------------------------------------------------------------

pub const KEY_VERSION: &[u8] = b"version";
pub const KEY_PAUSED: &[u8] = b"paused";
pub const KEY_OWNER: &[u8] = b"owner";
pub const KEY_DAILY_CAP: &[u8] = b"daily_cap";
pub const KEY_MIN_GAMES: &[u8] = b"min_games";
