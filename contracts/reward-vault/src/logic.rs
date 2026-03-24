//! Pure business logic for the Reward Vault.
//!
//! Every entry point in `lib.rs` (WASM target) delegates to these functions.
//! The same functions are called directly from integration tests via the
//! `MockEnv` shim, which allows full test coverage without a live VM.
//!
//! All functions take a mutable `MockEnv` reference so they can be tested
//! in isolation.  On the WASM target, the `wasm_entry` module in `lib.rs`
//! calls the equivalent logic inline against real host functions.
//!
//! # Checks-effects-interactions (CEI) pattern
//!
//! `apply_claim_reward` and `apply_withdraw` write all state changes before
//! calling `env.transfer()`.  If the transfer fails, the Err propagates to
//! the caller (test assertion or entry-point `require!`).  In the WASM VM the
//! entire execution frame reverts on `abort`, so there is no partial-state
//! risk.  Tests verify that storage is updated before the transfer is
//! attempted by inspecting storage after a successful call.

use crate::mock::MockEnv;
use crate::types::{
    addr_to_hex, claimed_key, day_from_timestamp, nonce_key, platform_key, KEY_DAILY_CAP,
    KEY_MIN_GAMES, KEY_OWNER, KEY_PAUSED, KEY_VERSION,
};

// ---------------------------------------------------------------------------
// init
// ---------------------------------------------------------------------------

/// Initialise the vault.  Must be called exactly once after deployment.
///
/// # Panics
///
/// Panics (via `MockEnv::panic_msg`) if the contract has already been
/// initialised.
pub fn apply_init(
    env: &mut MockEnv,
    owner: [u8; 32],
    daily_cap: u128,
    min_games: u64,
    platforms: Vec<[u8; 32]>,
) {
    // Guard: only once.
    if env.storage_exists(KEY_VERSION) {
        MockEnv::panic_msg("already initialized");
    }

    // Store version.
    env.storage_set(
        KEY_VERSION,
        &borsh::to_vec(&1u32).expect("borsh"),
    );

    // Store owner.
    env.storage_set(
        KEY_OWNER,
        &borsh::to_vec(&owner).expect("borsh"),
    );

    // Store daily cap (little-endian u128).
    env.storage_set(KEY_DAILY_CAP, &daily_cap.to_le_bytes());

    // Store min_games (little-endian u64).
    env.storage_set(KEY_MIN_GAMES, &min_games.to_le_bytes());

    // Not paused.
    env.storage_set(KEY_PAUSED, &[0u8]);

    // Authorise platforms.
    for addr in &platforms {
        env.storage_set(&platform_key(addr), &[1u8]);
    }
}

// ---------------------------------------------------------------------------
// fund
// ---------------------------------------------------------------------------

/// Accept a CLAW deposit into the vault.
///
/// The value attached to the call is credited to the contract's balance via
/// the `MockEnv` (in production this is handled by the VM automatically).
pub fn apply_fund(env: &mut MockEnv) {
    let value = env.get_value();
    if value > 0 {
        env.credit_contract(value);
    }
}

// ---------------------------------------------------------------------------
// claim_reward
// ---------------------------------------------------------------------------

/// Core payout function.  Returns `Ok(())` on success or an `Err(String)`
/// describing why the claim was rejected.
///
/// Security invariants enforced here:
/// 1. Contract must not be paused.
/// 2. Caller must be an authorised platform.
/// 3. Nonce must match exactly (anti-replay).
/// 4. `already_claimed + amount` must not exceed the daily cap.
/// 5. Contract must hold at least `amount` tokens.
///
/// State is written (effects) before the transfer (interaction) — CEI pattern.
pub fn apply_claim_reward(
    env: &mut MockEnv,
    recipient: [u8; 32],
    amount: u128,
    nonce: u64,
) -> Result<(), String> {
    // --- CHECKS ---

    // 1. Paused?
    if env
        .storage_get(KEY_PAUSED)
        .map(|b| b.first().copied().unwrap_or(0))
        .unwrap_or(0)
        == 1
    {
        return Err("paused: contract is paused".into());
    }

    // 2. Authorised platform?
    let caller = env.get_caller();
    let pkey = platform_key(&caller);
    let is_platform = env
        .storage_get(&pkey)
        .map(|b| b.first().copied().unwrap_or(0))
        .unwrap_or(0);
    if is_platform != 1 {
        return Err(format!(
            "unauthorized: {} is not an authorised platform",
            addr_to_hex(&caller)
        ));
    }

    // 3. Nonce.
    let stored_nonce = env
        .storage_get(&nonce_key(&recipient))
        .map(|b| u64::from_le_bytes(b[..8].try_into().unwrap()))
        .unwrap_or(0);
    if stored_nonce != nonce {
        return Err(format!(
            "nonce mismatch: expected {}, got {}",
            stored_nonce, nonce
        ));
    }

    // 4. Daily cap.
    let ts = env.get_timestamp();
    let day = day_from_timestamp(ts);
    let ckey = claimed_key(&recipient, day);
    let already_claimed = env
        .storage_get(&ckey)
        .map(|b| u128::from_le_bytes(b[..16].try_into().unwrap()))
        .unwrap_or(0);
    let daily_cap = env
        .storage_get(KEY_DAILY_CAP)
        .map(|b| u128::from_le_bytes(b[..16].try_into().unwrap()))
        .unwrap_or(0);

    // Use checked_add to guard against u128 overflow.
    let new_total = already_claimed
        .checked_add(amount)
        .ok_or_else(|| "daily cap: arithmetic overflow".to_string())?;
    if new_total > daily_cap {
        return Err(format!(
            "daily cap exceeded: {} + {} > {}",
            already_claimed, amount, daily_cap
        ));
    }

    // 5. Balance.
    let contract_addr = env.get_contract_address();
    let balance = env.get_balance(&contract_addr);
    if balance < amount {
        return Err(format!(
            "insufficient vault balance: {} < {}",
            balance, amount
        ));
    }

    // --- EFFECTS (write before transfer — CEI) ---

    env.storage_set(&ckey, &new_total.to_le_bytes());
    env.storage_set(&nonce_key(&recipient), &(stored_nonce + 1).to_le_bytes());

    // --- INTERACTIONS ---

    env.transfer(&recipient, amount)
        .map_err(|e| format!("token transfer failed: {}", e))?;

    Ok(())
}

// ---------------------------------------------------------------------------
// set_daily_cap
// ---------------------------------------------------------------------------

/// Update the daily cap.  Only the owner may call this.
pub fn apply_set_daily_cap(env: &mut MockEnv, new_cap: u128) {
    require_owner(env);
    env.storage_set(KEY_DAILY_CAP, &new_cap.to_le_bytes());
}

// ---------------------------------------------------------------------------
// add_platform / remove_platform
// ---------------------------------------------------------------------------

/// Authorise a new platform address.  Owner only.
pub fn apply_add_platform(env: &mut MockEnv, addr: [u8; 32]) {
    require_owner(env);
    env.storage_set(&platform_key(&addr), &[1u8]);
}

/// Revoke a platform's authorisation.  Owner only.
pub fn apply_remove_platform(env: &mut MockEnv, addr: [u8; 32]) {
    require_owner(env);
    env.storage_remove(&platform_key(&addr));
}

// ---------------------------------------------------------------------------
// pause / unpause
// ---------------------------------------------------------------------------

/// Halt all claims.  Owner only.
pub fn apply_pause(env: &mut MockEnv) {
    require_owner(env);
    env.storage_set(KEY_PAUSED, &[1u8]);
}

/// Resume claims.  Owner only.
pub fn apply_unpause(env: &mut MockEnv) {
    require_owner(env);
    env.storage_set(KEY_PAUSED, &[0u8]);
}

// ---------------------------------------------------------------------------
// withdraw
// ---------------------------------------------------------------------------

/// Emergency withdrawal to the owner's address.  Owner only.
pub fn apply_withdraw(env: &mut MockEnv, amount: u128) -> Result<(), String> {
    require_owner(env);

    let contract_addr = env.get_contract_address();
    let balance = env.get_balance(&contract_addr);
    if balance < amount {
        return Err(format!(
            "insufficient vault balance: {} < {}",
            balance, amount
        ));
    }

    let owner = read_owner(env);
    env.transfer(&owner, amount)
        .map_err(|e| format!("withdraw transfer failed: {}", e))?;

    Ok(())
}

// ---------------------------------------------------------------------------
// get_daily_claimed (view)
// ---------------------------------------------------------------------------

/// Return the amount claimed today by `addr`.
pub fn apply_get_daily_claimed(env: &MockEnv, addr: [u8; 32]) -> u128 {
    let ts = env.get_timestamp();
    let day = day_from_timestamp(ts);
    let ckey = claimed_key(&addr, day);
    env.storage_get(&ckey)
        .map(|b| u128::from_le_bytes(b[..16].try_into().unwrap()))
        .unwrap_or(0)
}

// ---------------------------------------------------------------------------
// cleanup_claims
// ---------------------------------------------------------------------------

/// Delete daily claimed records older than `before_day`.  Owner only.
///
/// The caller supplies an explicit list of addresses to bound the work per
/// transaction and avoid unbounded iteration.
pub fn apply_cleanup_claims(env: &mut MockEnv, before_day: u32, addrs: Vec<[u8; 32]>) {
    require_owner(env);
    for addr in &addrs {
        for day in 0u64..(before_day as u64) {
            let k = claimed_key(addr, day);
            if env.storage_exists(&k) {
                env.storage_remove(&k);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Read the stored owner address.
fn read_owner(env: &MockEnv) -> [u8; 32] {
    env.storage_get(KEY_OWNER)
        .and_then(|b| borsh::from_slice::<[u8; 32]>(&b).ok())
        .expect("owner not set")
}

/// Panic if the current caller is not the owner.
fn require_owner(env: &MockEnv) {
    let caller = env.get_caller();
    let owner = read_owner(env);
    if caller != owner {
        MockEnv::panic_msg(&format!(
            "caller is not the owner: {}",
            addr_to_hex(&caller)
        ));
    }
}
