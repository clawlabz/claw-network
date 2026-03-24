//! Arena Pool — Game Wallet contract for ranked CLAW matches.
//!
//! Entry points (Wasm exports) delegate immediately to the pure [`logic`]
//! module which owns all business rules. This separation makes the logic
//! fully unit/integration-testable without a Wasm runtime.

extern crate alloc;

pub mod logic;
pub mod types;

// ------------------------------------------------------------------
// Wasm-only: entry points
// ------------------------------------------------------------------
#[cfg(target_arch = "wasm32")]
mod wasm_entry {
    use super::logic;
    use super::types::*;
    use claw_sdk::env;

    // ----------------------------------------------------------------
    // Expose the `alloc` symbol required by the ClawNetwork VM.
    // The SDK's setup_alloc! macro does the same thing but uses
    // std::alloc::alloc — which is fine on wasm32-unknown-unknown since
    // that target ships with std.
    // ----------------------------------------------------------------
    claw_sdk::setup_alloc!();

    // ------------------------------------------------------------------
    // Helpers — load/persist the full ContractState from storage
    // ------------------------------------------------------------------

    fn load_state() -> logic::ContractState {
        use borsh::BorshDeserialize;
        match env::storage_get(b"__state__") {
            Some(bytes) => {
                logic::ContractState::try_from_slice(&bytes).expect("state deserialize failed")
            }
            None => logic::ContractState::default(),
        }
    }

    fn save_state(state: &logic::ContractState) {
        let bytes = borsh::to_vec(state).expect("state serialize failed");
        env::storage_set(b"__state__", &bytes);
    }

    /// Flush pending token transfers produced by the logic layer.
    fn flush_transfers(transfers: Vec<([u8; 32], u128)>) {
        for (to, amount) in transfers {
            if amount > 0 {
                let ok = env::transfer(&to, amount);
                claw_sdk::require!(ok, "token_transfer failed");
            }
        }
    }

    // ------------------------------------------------------------------
    // Entry points
    // ------------------------------------------------------------------

    #[no_mangle]
    pub extern "C" fn init(args_ptr: i32, args_len: i32) {
        claw_sdk::entry!(args_ptr, args_len, |args: InitArgs| {
            let mut state = logic::ContractState::default();
            logic::apply_init(&mut state, args.owner, args.platform, args.fee_bps, args.burn_bps);
            save_state(&state);
            b"ok".to_vec()
        });
    }

    #[no_mangle]
    pub extern "C" fn deposit(args_ptr: i32, args_len: i32) {
        claw_sdk::entry!(args_ptr, args_len, |_args: DepositArgs| {
            let caller = env::get_caller();
            let value = env::get_value();
            let mut state = load_state();
            logic::apply_deposit(&mut state, caller, value);
            save_state(&state);
            b"ok".to_vec()
        });
    }

    #[no_mangle]
    pub extern "C" fn withdraw(args_ptr: i32, args_len: i32) {
        claw_sdk::entry!(args_ptr, args_len, |args: WithdrawArgs| {
            let caller = env::get_caller();
            let mut state = load_state();
            let transfers = logic::apply_withdraw(&mut state, caller, args.amount);
            save_state(&state);
            flush_transfers(transfers);
            b"ok".to_vec()
        });
    }

    #[no_mangle]
    pub extern "C" fn lock_entries(args_ptr: i32, args_len: i32) {
        claw_sdk::entry!(args_ptr, args_len, |args: LockEntriesArgs| {
            let caller = env::get_caller();
            let ts = env::get_block_timestamp();
            let mut state = load_state();
            logic::apply_lock_entries(
                &mut state,
                caller,
                args.game_hash,
                args.players,
                args.entry_fee,
                ts,
            );
            save_state(&state);
            b"ok".to_vec()
        });
    }

    #[no_mangle]
    pub extern "C" fn settle_game(args_ptr: i32, args_len: i32) {
        claw_sdk::entry!(args_ptr, args_len, |args: SettleGameArgs| {
            let caller = env::get_caller();
            let mut state = load_state();
            let transfers = logic::apply_settle_game(
                &mut state,
                caller,
                args.game_hash,
                args.winners,
                args.amounts,
            );
            save_state(&state);
            flush_transfers(transfers);
            b"ok".to_vec()
        });
    }

    #[no_mangle]
    pub extern "C" fn refund_game(args_ptr: i32, args_len: i32) {
        claw_sdk::entry!(args_ptr, args_len, |args: RefundGameArgs| {
            let caller = env::get_caller();
            let mut state = load_state();
            logic::apply_refund_game(&mut state, caller, args.game_hash);
            save_state(&state);
            b"ok".to_vec()
        });
    }

    #[no_mangle]
    pub extern "C" fn refund_game_emergency(args_ptr: i32, args_len: i32) {
        claw_sdk::entry!(args_ptr, args_len, |args: EmergencyRefundArgs| {
            let caller = env::get_caller();
            let now = env::get_block_timestamp();
            let mut state = load_state();
            logic::apply_emergency_refund(&mut state, caller, args.game_hash, now);
            save_state(&state);
            b"ok".to_vec()
        });
    }

    #[no_mangle]
    pub extern "C" fn claim_fees(args_ptr: i32, args_len: i32) {
        claw_sdk::entry!(args_ptr, args_len, |_args: ClaimFeesArgs| {
            let caller = env::get_caller();
            let mut state = load_state();
            let transfers = logic::apply_claim_fees(&mut state, caller);
            save_state(&state);
            flush_transfers(transfers);
            b"ok".to_vec()
        });
    }

    #[no_mangle]
    pub extern "C" fn pause(args_ptr: i32, args_len: i32) {
        claw_sdk::entry!(args_ptr, args_len, |_args: PauseArgs| {
            let caller = env::get_caller();
            let mut state = load_state();
            logic::apply_pause(&mut state, caller);
            save_state(&state);
            b"ok".to_vec()
        });
    }

    #[no_mangle]
    pub extern "C" fn unpause(args_ptr: i32, args_len: i32) {
        claw_sdk::entry!(args_ptr, args_len, |_args: PauseArgs| {
            let caller = env::get_caller();
            let mut state = load_state();
            logic::apply_unpause(&mut state, caller);
            save_state(&state);
            b"ok".to_vec()
        });
    }

    #[no_mangle]
    pub extern "C" fn cleanup_games(args_ptr: i32, args_len: i32) {
        claw_sdk::entry!(args_ptr, args_len, |args: CleanupGamesArgs| {
            let caller = env::get_caller();
            let mut state = load_state();
            logic::apply_cleanup_games(&mut state, caller, args.hashes);
            save_state(&state);
            b"ok".to_vec()
        });
    }
}
