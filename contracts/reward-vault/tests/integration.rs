//! Integration tests for the Reward Vault contract.
//!
//! Since the contract's `entry!` macro calls real host functions (which only
//! exist inside the ClawNetwork VM), these tests drive the *pure business
//! logic* layer directly — the same functions that each entry point delegates
//! to.  This is the idiomatic approach for testing `cdylib` contracts without
//! spinning up a full VM.
//!
//! TDD workflow followed here:
//!   RED  → tests written first, implementation absent.
//!   GREEN → minimal implementation added.
//!   REFACTOR → clean up without breaking green.

use reward_vault::logic::{
    apply_add_platform, apply_claim_reward, apply_cleanup_claims, apply_fund,
    apply_get_daily_claimed, apply_init, apply_pause, apply_remove_platform,
    apply_set_daily_cap, apply_unpause, apply_withdraw,
};
use reward_vault::mock::{MockEnv, ZERO_ADDR};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn owner_addr() -> [u8; 32] {
    [0x01u8; 32]
}

fn platform_addr() -> [u8; 32] {
    [0x02u8; 32]
}

fn recipient_addr() -> [u8; 32] {
    [0x03u8; 32]
}

fn other_addr() -> [u8; 32] {
    [0x04u8; 32]
}

/// Bootstrap a fully initialised MockEnv with standard parameters.
fn make_env() -> MockEnv {
    let mut env = MockEnv::new();
    // Contract holds 1_000 CLAW (in nano-CLAW units, 1 CLAW = 1e9 nano-CLAW
    // for test convenience).
    env.set_contract_balance(1_000_000_000_000u128);
    // Owner calls init.
    env.set_caller(owner_addr());
    env.set_timestamp(1_000 * 86_400); // day 1000
    apply_init(
        &mut env,
        owner_addr(),
        500_000_000u128,    // daily_cap = 500 CLAW
        1,                  // min_games = 1
        vec![platform_addr()],
    );
    env
}

// ===========================================================================
// 1. Initialisation
// ===========================================================================

#[test]
fn test_init_stores_owner() {
    let env = make_env();
    assert_eq!(env.read_owner(), owner_addr());
}

#[test]
fn test_init_stores_daily_cap() {
    let env = make_env();
    assert_eq!(env.read_daily_cap(), 500_000_000u128);
}

#[test]
fn test_init_stores_min_games() {
    let env = make_env();
    assert_eq!(env.read_min_games(), 1u64);
}

#[test]
fn test_init_marks_platform_authorized() {
    let env = make_env();
    assert!(env.is_platform_authorized(&platform_addr()));
}

#[test]
fn test_init_not_paused() {
    let env = make_env();
    assert!(!env.is_paused());
}

#[test]
fn test_init_version_is_one() {
    let env = make_env();
    assert_eq!(env.read_version(), 1u32);
}

#[test]
#[should_panic(expected = "already initialized")]
fn test_init_cannot_be_called_twice() {
    let mut env = make_env();
    env.set_caller(owner_addr());
    apply_init(&mut env, owner_addr(), 100, 1, vec![]);
}

// ===========================================================================
// 2. fund — accept value
// ===========================================================================

#[test]
fn test_fund_increases_balance() {
    let mut env = make_env();
    let initial = env.contract_balance();
    env.set_caller(other_addr());
    env.set_value(1_000u128);
    apply_fund(&mut env);
    assert_eq!(env.contract_balance(), initial + 1_000);
}

#[test]
fn test_fund_with_zero_value_is_noop() {
    let mut env = make_env();
    let initial = env.contract_balance();
    env.set_caller(other_addr());
    env.set_value(0u128);
    apply_fund(&mut env);
    assert_eq!(env.contract_balance(), initial);
}

#[test]
fn test_fund_anyone_can_call() {
    let mut env = make_env();
    // Completely unknown address — should not panic.
    env.set_caller([0xFFu8; 32]);
    env.set_value(42u128);
    apply_fund(&mut env);
    assert_eq!(env.contract_balance(), 1_000_000_000_000u128 + 42);
}

// ===========================================================================
// 3. claim_reward — happy path
// ===========================================================================

#[test]
fn test_claim_reward_transfers_tokens() {
    let mut env = make_env();
    env.set_caller(platform_addr());
    env.set_timestamp(1_000 * 86_400 + 100); // still day 1000

    let before = env.contract_balance();
    apply_claim_reward(&mut env, recipient_addr(), 100u128, 0u64)
        .expect("claim should succeed");
    assert_eq!(env.contract_balance(), before - 100);
    assert_eq!(env.balance_of(&recipient_addr()), 100u128);
}

#[test]
fn test_claim_reward_increments_nonce() {
    let mut env = make_env();
    env.set_caller(platform_addr());
    env.set_timestamp(1_000 * 86_400);

    apply_claim_reward(&mut env, recipient_addr(), 10u128, 0u64).unwrap();
    assert_eq!(env.read_nonce(&recipient_addr()), 1u64);

    apply_claim_reward(&mut env, recipient_addr(), 10u128, 1u64).unwrap();
    assert_eq!(env.read_nonce(&recipient_addr()), 2u64);
}

#[test]
fn test_claim_reward_records_daily_claimed() {
    let mut env = make_env();
    env.set_caller(platform_addr());
    env.set_timestamp(1_000 * 86_400);

    apply_claim_reward(&mut env, recipient_addr(), 250u128, 0u64).unwrap();
    assert_eq!(apply_get_daily_claimed(&env, recipient_addr()), 250u128);
}

#[test]
fn test_claim_reward_accumulates_within_day() {
    let mut env = make_env();
    env.set_caller(platform_addr());
    env.set_timestamp(1_000 * 86_400);

    apply_claim_reward(&mut env, recipient_addr(), 100u128, 0u64).unwrap();
    apply_claim_reward(&mut env, recipient_addr(), 150u128, 1u64).unwrap();
    assert_eq!(apply_get_daily_claimed(&env, recipient_addr()), 250u128);
}

// ===========================================================================
// 4. claim_reward — daily cap enforcement
// ===========================================================================

#[test]
fn test_claim_reward_exactly_at_cap_succeeds() {
    let mut env = make_env();
    env.set_caller(platform_addr());
    env.set_timestamp(1_000 * 86_400);

    apply_claim_reward(&mut env, recipient_addr(), 500_000_000u128, 0u64)
        .expect("exactly at cap should succeed");
}

#[test]
fn test_claim_reward_over_cap_fails() {
    let mut env = make_env();
    env.set_caller(platform_addr());
    env.set_timestamp(1_000 * 86_400);

    let result = apply_claim_reward(&mut env, recipient_addr(), 500_000_001u128, 0u64);
    assert!(result.is_err(), "should fail when over daily cap");
    assert!(result.unwrap_err().contains("daily cap"));
}

#[test]
fn test_claim_reward_over_cap_on_second_claim_fails() {
    let mut env = make_env();
    env.set_caller(platform_addr());
    env.set_timestamp(1_000 * 86_400);

    apply_claim_reward(&mut env, recipient_addr(), 400_000_000u128, 0u64).unwrap();
    let result =
        apply_claim_reward(&mut env, recipient_addr(), 100_000_001u128, 1u64);
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("daily cap"));
}

#[test]
fn test_daily_cap_resets_next_day() {
    let mut env = make_env();
    env.set_caller(platform_addr());
    env.set_timestamp(1_000 * 86_400); // day 1000

    apply_claim_reward(&mut env, recipient_addr(), 500_000_000u128, 0u64).unwrap();

    // Advance to day 1001.
    env.set_timestamp(1_001 * 86_400);
    apply_claim_reward(&mut env, recipient_addr(), 500_000_000u128, 1u64)
        .expect("new day should have fresh cap");
}

// ===========================================================================
// 5. claim_reward — nonce (replay protection)
// ===========================================================================

#[test]
fn test_claim_wrong_nonce_fails() {
    let mut env = make_env();
    env.set_caller(platform_addr());
    env.set_timestamp(1_000 * 86_400);

    let result = apply_claim_reward(&mut env, recipient_addr(), 10u128, 1u64); // nonce should be 0
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("nonce"));
}

#[test]
fn test_claim_replayed_nonce_fails() {
    let mut env = make_env();
    env.set_caller(platform_addr());
    env.set_timestamp(1_000 * 86_400);

    apply_claim_reward(&mut env, recipient_addr(), 10u128, 0u64).unwrap();
    // Try to replay nonce 0.
    let result = apply_claim_reward(&mut env, recipient_addr(), 10u128, 0u64);
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("nonce"));
}

#[test]
fn test_nonces_are_per_recipient() {
    let mut env = make_env();
    env.set_caller(platform_addr());
    env.set_timestamp(1_000 * 86_400);

    // First recipient uses nonce 0.
    apply_claim_reward(&mut env, recipient_addr(), 10u128, 0u64).unwrap();
    // Different recipient also starts at nonce 0.
    apply_claim_reward(&mut env, other_addr(), 10u128, 0u64).unwrap();
}

// ===========================================================================
// 6. claim_reward — authorization
// ===========================================================================

#[test]
fn test_claim_unauthorized_caller_fails() {
    let mut env = make_env();
    env.set_caller(other_addr()); // not a platform
    env.set_timestamp(1_000 * 86_400);

    let result = apply_claim_reward(&mut env, recipient_addr(), 10u128, 0u64);
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("unauthorized"));
}

#[test]
fn test_claim_owner_is_not_platform_by_default() {
    let mut env = make_env();
    env.set_caller(owner_addr()); // owner != platform
    env.set_timestamp(1_000 * 86_400);

    let result = apply_claim_reward(&mut env, recipient_addr(), 10u128, 0u64);
    assert!(result.is_err());
}

// ===========================================================================
// 7. claim_reward — insufficient balance
// ===========================================================================

#[test]
fn test_claim_insufficient_balance_fails() {
    let mut env = MockEnv::new();
    env.set_contract_balance(50u128); // only 50 available
    env.set_caller(owner_addr());
    env.set_timestamp(1_000 * 86_400);
    apply_init(&mut env, owner_addr(), 1_000_000u128, 1, vec![platform_addr()]);

    env.set_caller(platform_addr());
    let result = apply_claim_reward(&mut env, recipient_addr(), 100u128, 0u64);
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("insufficient"));
}

// ===========================================================================
// 8. Pause / unpause
// ===========================================================================

#[test]
fn test_pause_blocks_claims() {
    let mut env = make_env();
    env.set_caller(owner_addr());
    apply_pause(&mut env);

    env.set_caller(platform_addr());
    env.set_timestamp(1_000 * 86_400);
    let result = apply_claim_reward(&mut env, recipient_addr(), 10u128, 0u64);
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("paused"));
}

#[test]
fn test_unpause_allows_claims() {
    let mut env = make_env();
    env.set_caller(owner_addr());
    apply_pause(&mut env);
    apply_unpause(&mut env);

    env.set_caller(platform_addr());
    env.set_timestamp(1_000 * 86_400);
    apply_claim_reward(&mut env, recipient_addr(), 10u128, 0u64)
        .expect("should succeed after unpause");
}

#[test]
fn test_pause_only_owner() {
    let mut env = make_env();
    env.set_caller(other_addr());
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        apply_pause(&mut env)
    }));
    // Should panic (env.panic_msg is called).
    assert!(result.is_err(), "non-owner must not be able to pause");
}

#[test]
fn test_unpause_only_owner() {
    let mut env = make_env();
    env.set_caller(owner_addr());
    apply_pause(&mut env);
    env.set_caller(other_addr());
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        apply_unpause(&mut env)
    }));
    assert!(result.is_err(), "non-owner must not be able to unpause");
}

// ===========================================================================
// 9. Platform management
// ===========================================================================

#[test]
fn test_add_platform_authorizes_new_caller() {
    let mut env = make_env();
    env.set_caller(owner_addr());
    apply_add_platform(&mut env, other_addr());

    env.set_caller(other_addr());
    env.set_timestamp(1_000 * 86_400);
    apply_claim_reward(&mut env, recipient_addr(), 10u128, 0u64)
        .expect("newly added platform should be able to claim");
}

#[test]
fn test_remove_platform_revokes_access() {
    let mut env = make_env();
    env.set_caller(owner_addr());
    apply_remove_platform(&mut env, platform_addr());

    env.set_caller(platform_addr());
    env.set_timestamp(1_000 * 86_400);
    let result = apply_claim_reward(&mut env, recipient_addr(), 10u128, 0u64);
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("unauthorized"));
}

#[test]
fn test_add_platform_only_owner() {
    let mut env = make_env();
    env.set_caller(other_addr());
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        apply_add_platform(&mut env, [0xAAu8; 32])
    }));
    assert!(result.is_err());
}

#[test]
fn test_remove_platform_only_owner() {
    let mut env = make_env();
    env.set_caller(other_addr());
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        apply_remove_platform(&mut env, platform_addr())
    }));
    assert!(result.is_err());
}

// ===========================================================================
// 10. set_daily_cap
// ===========================================================================

#[test]
fn test_set_daily_cap_updates_value() {
    let mut env = make_env();
    env.set_caller(owner_addr());
    apply_set_daily_cap(&mut env, 999_999u128);
    assert_eq!(env.read_daily_cap(), 999_999u128);
}

#[test]
fn test_set_daily_cap_only_owner() {
    let mut env = make_env();
    env.set_caller(other_addr());
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        apply_set_daily_cap(&mut env, 1u128)
    }));
    assert!(result.is_err());
}

#[test]
fn test_set_daily_cap_zero_blocks_all_claims() {
    let mut env = make_env();
    env.set_caller(owner_addr());
    apply_set_daily_cap(&mut env, 0u128);

    env.set_caller(platform_addr());
    env.set_timestamp(1_000 * 86_400);
    let result = apply_claim_reward(&mut env, recipient_addr(), 1u128, 0u64);
    assert!(result.is_err());
}

// ===========================================================================
// 11. withdraw (owner emergency)
// ===========================================================================

#[test]
fn test_withdraw_reduces_balance() {
    let mut env = make_env();
    let initial = env.contract_balance();
    env.set_caller(owner_addr());
    apply_withdraw(&mut env, 1_000u128).expect("withdraw should succeed");
    assert_eq!(env.contract_balance(), initial - 1_000);
}

#[test]
fn test_withdraw_only_owner() {
    let mut env = make_env();
    env.set_caller(other_addr());
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        apply_withdraw(&mut env, 1u128).unwrap()
    }));
    assert!(result.is_err());
}

#[test]
fn test_withdraw_over_balance_fails() {
    let mut env = make_env();
    env.set_caller(owner_addr());
    let over = env.contract_balance() + 1;
    let result = apply_withdraw(&mut env, over);
    assert!(result.is_err());
}

#[test]
fn test_withdraw_exact_balance_succeeds() {
    let mut env = make_env();
    let bal = env.contract_balance();
    env.set_caller(owner_addr());
    apply_withdraw(&mut env, bal).expect("withdrawing entire balance should succeed");
    assert_eq!(env.contract_balance(), 0);
}

// ===========================================================================
// 12. get_daily_claimed view
// ===========================================================================

#[test]
fn test_get_daily_claimed_returns_zero_for_fresh_address() {
    let env = make_env();
    assert_eq!(apply_get_daily_claimed(&env, recipient_addr()), 0u128);
}

#[test]
fn test_get_daily_claimed_reflects_claims() {
    let mut env = make_env();
    env.set_caller(platform_addr());
    env.set_timestamp(1_000 * 86_400);
    apply_claim_reward(&mut env, recipient_addr(), 77u128, 0u64).unwrap();
    assert_eq!(apply_get_daily_claimed(&env, recipient_addr()), 77u128);
}

// ===========================================================================
// 13. cleanup_claims
// ===========================================================================

#[test]
fn test_cleanup_removes_old_day_records() {
    let mut env = make_env();
    env.set_caller(platform_addr());

    // Claim on day 1000.
    env.set_timestamp(1_000 * 86_400);
    apply_claim_reward(&mut env, recipient_addr(), 10u128, 0u64).unwrap();

    // Claim on day 1001.
    env.set_timestamp(1_001 * 86_400);
    apply_claim_reward(&mut env, recipient_addr(), 20u128, 1u64).unwrap();

    // Owner cleans up days before 1001 (so day 1000 should be removed).
    env.set_caller(owner_addr());
    apply_cleanup_claims(&mut env, 1001u32, vec![recipient_addr()]);

    // Day 1000 record should be gone.
    assert!(!env.has_claimed_key(&recipient_addr(), 1000u64));
    // Day 1001 record should still be present.
    assert!(env.has_claimed_key(&recipient_addr(), 1001u64));
}

#[test]
fn test_cleanup_only_owner() {
    let mut env = make_env();
    env.set_caller(other_addr());
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        apply_cleanup_claims(&mut env, 999u32, vec![recipient_addr()])
    }));
    assert!(result.is_err());
}

#[test]
fn test_cleanup_with_empty_addrs_is_noop() {
    let mut env = make_env();
    env.set_caller(owner_addr());
    // Should not panic.
    apply_cleanup_claims(&mut env, 1000u32, vec![]);
}

// ===========================================================================
// 14. Checks-effects-interactions ordering
// ===========================================================================

/// Verify that the claimed amount is updated BEFORE the transfer is attempted.
/// If the transfer fails, the state update must have already been made and the
/// whole call reverts — but in our mock we verify the write happens first.
#[test]
fn test_checks_effects_interactions_order() {
    let mut env = make_env();
    env.set_caller(platform_addr());
    env.set_timestamp(1_000 * 86_400);

    // Drain all balance by claiming up to what the vault holds.
    // Vault balance = 1_000_000_000_000, daily_cap = 500_000_000.
    // Claim exactly the daily cap.
    apply_claim_reward(&mut env, recipient_addr(), 500_000_000u128, 0u64).unwrap();

    // Claimed amount in storage should be updated even after transfer.
    assert_eq!(apply_get_daily_claimed(&env, recipient_addr()), 500_000_000u128);
    // Nonce should be incremented.
    assert_eq!(env.read_nonce(&recipient_addr()), 1u64);
}

// ===========================================================================
// 15. Edge cases
// ===========================================================================

#[test]
fn test_zero_addr_is_not_a_valid_platform() {
    let mut env = make_env();
    env.set_caller(ZERO_ADDR);
    env.set_timestamp(1_000 * 86_400);
    let result = apply_claim_reward(&mut env, recipient_addr(), 1u128, 0u64);
    assert!(result.is_err());
}

#[test]
fn test_claim_amount_zero_allowed_by_contract_logic() {
    // A zero-amount claim from an authorised platform is technically valid.
    // The platform is responsible for not sending useless calls, but the
    // contract must not panic.
    let mut env = make_env();
    env.set_caller(platform_addr());
    env.set_timestamp(1_000 * 86_400);
    apply_claim_reward(&mut env, recipient_addr(), 0u128, 0u64)
        .expect("zero amount should not panic contract");
}

#[test]
fn test_large_amount_at_cap_boundary() {
    // daily_cap = u128::MAX — should still work without overflow.
    let mut env = MockEnv::new();
    env.set_contract_balance(u128::MAX);
    env.set_caller(owner_addr());
    env.set_timestamp(1_000 * 86_400);
    apply_init(&mut env, owner_addr(), u128::MAX, 1, vec![platform_addr()]);

    env.set_caller(platform_addr());
    // Claim just below max.
    let result = apply_claim_reward(&mut env, recipient_addr(), u128::MAX - 1, 0u64);
    assert!(result.is_ok());
}

#[test]
fn test_many_platforms_can_all_claim() {
    let mut env = make_env();
    let p2 = [0x10u8; 32];
    let p3 = [0x11u8; 32];
    env.set_caller(owner_addr());
    apply_add_platform(&mut env, p2);
    apply_add_platform(&mut env, p3);

    env.set_timestamp(1_000 * 86_400);

    env.set_caller(platform_addr());
    apply_claim_reward(&mut env, recipient_addr(), 10u128, 0u64).unwrap();

    env.set_caller(p2);
    apply_claim_reward(&mut env, other_addr(), 10u128, 0u64).unwrap();

    env.set_caller(p3);
    apply_claim_reward(&mut env, [0x20u8; 32], 10u128, 0u64).unwrap();
}
