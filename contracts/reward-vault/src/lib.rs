//! Reward Vault — ClawNetwork smart contract.
//!
//! Holds CLAW and distributes daily rewards to agents. Authorised platform
//! callers trigger payouts; an owner controls parameters and emergency ops.
//!
//! # Security model
//!
//! - **Anti-replay**: per-recipient monotonic nonce stored on-chain.
//! - **Daily cap**: total payout per recipient per UTC day is bounded.
//! - **Authorisation**: only registered platform addresses can call
//!   `claim_reward`; owner governs the platform list.
//! - **Checks-effects-interactions**: state written before any token transfer.
//! - **Pause circuit-breaker**: owner can halt all claims instantly.
//!
//! # Build targets
//!
//! - `wasm32-unknown-unknown`: the deployable contract binary.  Uses
//!   `no_std` + `alloc`.
//! - Native (any other target): compiles `logic` and `mock` modules so that
//!   `cargo test` works without a VM.

// On wasm32 we have no std, only alloc.
#![cfg_attr(target_arch = "wasm32", no_std)]
extern crate alloc;

pub mod types;

// Logic and mock modules are only compiled for testing / non-wasm targets.
#[cfg(not(target_arch = "wasm32"))]
pub mod logic;
#[cfg(not(target_arch = "wasm32"))]
pub mod mock;

// ---------------------------------------------------------------------------
// WASM entry points (only compiled for the wasm32 target)
// ---------------------------------------------------------------------------
#[cfg(target_arch = "wasm32")]
mod wasm_entry {
    use alloc::{format, vec, vec::Vec};

    use claw_sdk::{entry, env, require, storage};

    use crate::types::{
        addr_to_hex, claimed_key, day_from_timestamp, nonce_key, platform_key, ClaimRewardArgs,
        CleanupClaimsArgs, FundArgs, GetDailyClaimedArgs, InitArgs, PlatformArgs, SetDailyCapArgs,
        WithdrawArgs, KEY_DAILY_CAP, KEY_MIN_GAMES, KEY_OWNER, KEY_PAUSED, KEY_VERSION,
    };

    // Provide the VM-required `alloc` export.
    // We inline this instead of using `setup_alloc!()` because the SDK macro
    // references `std::alloc` which is not available in `no_std` crates.
    #[no_mangle]
    pub extern "C" fn alloc(size: i32) -> *mut u8 {
        let layout = core::alloc::Layout::from_size_align(size as usize, 1).unwrap();
        unsafe { alloc::alloc::alloc(layout) }
    }

    // -----------------------------------------------------------------------
    // init
    // -----------------------------------------------------------------------

    #[no_mangle]
    pub extern "C" fn init(args_ptr: i32, args_len: i32) {
        entry!(args_ptr, args_len, |args: InitArgs| {
            require!(
                storage::get::<u32>(KEY_VERSION).is_none(),
                "already initialized"
            );

            storage::set::<u32>(KEY_VERSION, &1u32);
            storage::set(KEY_OWNER, &args.owner);
            storage::set_u128(KEY_DAILY_CAP, args.daily_cap);
            storage::set_u64(KEY_MIN_GAMES, args.min_games);
            env::storage_set(KEY_PAUSED, &[0u8]);

            for addr in &args.platforms {
                env::storage_set(&platform_key(addr), &[1u8]);
            }

            env::log("reward-vault: initialized");
            vec![]
        });
    }

    // -----------------------------------------------------------------------
    // claim_reward
    // -----------------------------------------------------------------------

    #[no_mangle]
    pub extern "C" fn claim_reward(args_ptr: i32, args_len: i32) {
        entry!(args_ptr, args_len, |args: ClaimRewardArgs| {
            // --- CHECKS ---

            // 1. Not paused.
            let paused = env::storage_get(KEY_PAUSED)
                .map(|b| b.first().copied().unwrap_or(0))
                .unwrap_or(0);
            require!(paused == 0, "paused");

            // 2. Caller is authorised platform.
            let caller = env::get_caller();
            let pkey = platform_key(&caller);
            let is_platform = env::storage_get(&pkey)
                .map(|b| b.first().copied().unwrap_or(0))
                .unwrap_or(0);
            require!(is_platform == 1, "unauthorized: caller is not a platform");

            // 3. Nonce matches.
            let stored_nonce = storage::get_u64(&nonce_key(&args.recipient)).unwrap_or(0);
            require!(stored_nonce == args.nonce, "nonce mismatch");

            // 4. Daily cap not exceeded.
            let ts = env::get_block_timestamp();
            let day = day_from_timestamp(ts);
            let ckey = claimed_key(&args.recipient, day);
            let already_claimed = storage::get_u128(&ckey).unwrap_or(0);
            let daily_cap = storage::get_u128(KEY_DAILY_CAP).unwrap_or(0);
            let new_total = already_claimed
                .checked_add(args.amount)
                .unwrap_or(u128::MAX);
            require!(new_total <= daily_cap, "daily cap exceeded");

            // 5. Contract has sufficient balance.
            let self_addr = env::get_contract_address();
            let balance = env::get_balance(&self_addr);
            require!(balance >= args.amount, "insufficient vault balance");

            // --- EFFECTS (write state before transfer — CEI) ---

            storage::set_u128(&ckey, already_claimed + args.amount);
            storage::set_u64(&nonce_key(&args.recipient), stored_nonce + 1);

            // --- INTERACTIONS ---

            let ok = env::transfer(&args.recipient, args.amount);
            require!(ok, "token transfer failed");

            env::log(&format!(
                "reward-vault: claimed {} to {} nonce={}",
                args.amount,
                addr_to_hex(&args.recipient),
                args.nonce
            ));

            vec![]
        });
    }

    // -----------------------------------------------------------------------
    // fund
    // -----------------------------------------------------------------------

    #[no_mangle]
    pub extern "C" fn fund(args_ptr: i32, args_len: i32) {
        entry!(args_ptr, args_len, |_args: FundArgs| {
            let value = env::get_value();
            env::log(&format!("reward-vault: funded with {}", value));
            vec![]
        });
    }

    // -----------------------------------------------------------------------
    // set_daily_cap
    // -----------------------------------------------------------------------

    #[no_mangle]
    pub extern "C" fn set_daily_cap(args_ptr: i32, args_len: i32) {
        entry!(args_ptr, args_len, |args: SetDailyCapArgs| {
            require_owner!();
            storage::set_u128(KEY_DAILY_CAP, args.new_cap);
            env::log(&format!("reward-vault: daily_cap set to {}", args.new_cap));
            vec![]
        });
    }

    // -----------------------------------------------------------------------
    // add_platform / remove_platform
    // -----------------------------------------------------------------------

    #[no_mangle]
    pub extern "C" fn add_platform(args_ptr: i32, args_len: i32) {
        entry!(args_ptr, args_len, |args: PlatformArgs| {
            require_owner!();
            env::storage_set(&platform_key(&args.addr), &[1u8]);
            env::log(&format!(
                "reward-vault: platform added {}",
                addr_to_hex(&args.addr)
            ));
            vec![]
        });
    }

    #[no_mangle]
    pub extern "C" fn remove_platform(args_ptr: i32, args_len: i32) {
        entry!(args_ptr, args_len, |args: PlatformArgs| {
            require_owner!();
            env::storage_remove(&platform_key(&args.addr));
            env::log(&format!(
                "reward-vault: platform removed {}",
                addr_to_hex(&args.addr)
            ));
            vec![]
        });
    }

    // -----------------------------------------------------------------------
    // pause / unpause
    // -----------------------------------------------------------------------

    #[no_mangle]
    pub extern "C" fn pause(args_ptr: i32, args_len: i32) {
        entry!(args_ptr, args_len, |_args: FundArgs| {
            require_owner!();
            env::storage_set(KEY_PAUSED, &[1u8]);
            env::log("reward-vault: paused");
            vec![]
        });
    }

    #[no_mangle]
    pub extern "C" fn unpause(args_ptr: i32, args_len: i32) {
        entry!(args_ptr, args_len, |_args: FundArgs| {
            require_owner!();
            env::storage_set(KEY_PAUSED, &[0u8]);
            env::log("reward-vault: unpaused");
            vec![]
        });
    }

    // -----------------------------------------------------------------------
    // withdraw
    // -----------------------------------------------------------------------

    #[no_mangle]
    pub extern "C" fn withdraw(args_ptr: i32, args_len: i32) {
        entry!(args_ptr, args_len, |args: WithdrawArgs| {
            require_owner!();
            let self_addr = env::get_contract_address();
            let balance = env::get_balance(&self_addr);
            require!(balance >= args.amount, "insufficient vault balance");
            let owner: [u8; 32] = storage::get(KEY_OWNER).expect("no owner");
            let ok = env::transfer(&owner, args.amount);
            require!(ok, "token transfer failed");
            env::log(&format!("reward-vault: withdrew {}", args.amount));
            vec![]
        });
    }

    // -----------------------------------------------------------------------
    // get_daily_claimed (view)
    // -----------------------------------------------------------------------

    #[no_mangle]
    pub extern "C" fn get_daily_claimed(args_ptr: i32, args_len: i32) {
        entry!(args_ptr, args_len, |args: GetDailyClaimedArgs| {
            let ts = env::get_block_timestamp();
            let day = day_from_timestamp(ts);
            let ckey = claimed_key(&args.addr, day);
            let claimed = storage::get_u128(&ckey).unwrap_or(0);
            borsh::to_vec(&claimed).unwrap()
        });
    }

    // -----------------------------------------------------------------------
    // cleanup_claims
    // -----------------------------------------------------------------------

    #[no_mangle]
    pub extern "C" fn cleanup_claims(args_ptr: i32, args_len: i32) {
        entry!(args_ptr, args_len, |args: CleanupClaimsArgs| {
            require_owner!();
            require!(args.before_day <= 365, "before_day too large — max 365");
            require!(args.addrs.len() <= 50, "too many addresses — max 50");
            for addr in &args.addrs {
                for day in 0u64..(args.before_day as u64) {
                    let k = claimed_key(addr, day);
                    if env::storage_exists(&k) {
                        env::storage_remove(&k);
                    }
                }
            }
            env::log(&format!(
                "reward-vault: cleaned up days before {}",
                args.before_day
            ));
            vec![]
        });
    }

    // -----------------------------------------------------------------------
    // Internal helper macro (owner guard)
    // -----------------------------------------------------------------------

    macro_rules! require_owner {
        () => {{
            let caller = env::get_caller();
            let owner: [u8; 32] = storage::get(KEY_OWNER).expect("no owner");
            require!(caller == owner, "caller is not the owner");
        }};
    }
    use require_owner;
}
