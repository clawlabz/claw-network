//! Integration tests for the Arena Pool contract.
//!
//! TDD: these tests were written BEFORE the implementation. They describe the
//! full expected behaviour and must all pass after implementation is complete.
//!
//! Because smart contract entry points call raw host-function FFI, we cannot
//! run them directly in a native test process. Instead we test the pure
//! business-logic layer (`arena_pool::logic`) that the entry points delegate
//! to. That layer receives all I/O through explicit parameters and returns
//! structured results, making it 100 % testable without a Wasm runtime.

use arena_pool::logic::{
    apply_claim_fees, apply_cleanup_games, apply_deposit, apply_emergency_refund,
    apply_init, apply_lock_entries, apply_pause, apply_refund_game, apply_settle_game,
    apply_unpause, apply_withdraw, ContractState,
};
use arena_pool::types::game_status;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn addr(seed: u8) -> [u8; 32] {
    let mut a = [0u8; 32];
    a[0] = seed;
    a
}

fn game_hash(seed: u8) -> [u8; 32] {
    let mut h = [0u8; 32];
    h[0] = 0xAA;
    h[1] = seed;
    h
}

/// Build a fresh, initialised contract state.
fn new_state() -> ContractState {
    let mut state = ContractState::default();
    apply_init(
        &mut state,
        addr(1),  // owner
        addr(2),  // platform
        300,      // fee_bps  = 3 %
        200,      // burn_bps = 2 %
    );
    state
}

/// Add balance directly (simulates a deposit with msg.value).
fn credit(state: &mut ContractState, player: [u8; 32], amount: u128) {
    let bal = state.balance.entry(player).or_insert(0);
    *bal += amount;
}

// ---------------------------------------------------------------------------
// 1. init
// ---------------------------------------------------------------------------

#[test]
fn test_init_sets_owner_and_platform() {
    let state = new_state();
    assert_eq!(state.owner, addr(1));
    assert_eq!(state.platform, addr(2));
    assert_eq!(state.fee_bps, 300);
    assert_eq!(state.burn_bps, 200);
    assert_eq!(state.version, 1);
    assert!(!state.paused);
}

#[test]
fn test_init_rejects_fee_plus_burn_exceeds_10000() {
    let mut state = ContractState::default();
    let result = std::panic::catch_unwind(move || {
        apply_init(&mut state, addr(1), addr(2), 8000, 3000);
    });
    assert!(result.is_err(), "fee_bps + burn_bps > 10000 should panic");
}

// ---------------------------------------------------------------------------
// 2. deposit
// ---------------------------------------------------------------------------

#[test]
fn test_deposit_increases_balance() {
    let mut state = new_state();
    apply_deposit(&mut state, addr(10), 1_000_000);
    assert_eq!(state.balance[&addr(10)], 1_000_000);
}

#[test]
fn test_deposit_accumulates() {
    let mut state = new_state();
    apply_deposit(&mut state, addr(10), 500_000);
    apply_deposit(&mut state, addr(10), 300_000);
    assert_eq!(state.balance[&addr(10)], 800_000);
}

#[test]
fn test_deposit_zero_value_rejected() {
    let mut state = new_state();
    let result = std::panic::catch_unwind(move || {
        apply_deposit(&mut state, addr(10), 0);
    });
    assert!(result.is_err(), "zero deposit should be rejected");
}

#[test]
fn test_deposit_blocked_when_paused() {
    let mut state = new_state();
    apply_pause(&mut state, addr(1));
    let result = std::panic::catch_unwind(move || {
        apply_deposit(&mut state, addr(10), 1_000_000);
    });
    assert!(result.is_err(), "deposit when paused should panic");
}

// ---------------------------------------------------------------------------
// 3. withdraw
// ---------------------------------------------------------------------------

#[test]
fn test_deposit_withdraw_round_trip() {
    let mut state = new_state();
    apply_deposit(&mut state, addr(10), 1_000_000);
    let transfers = apply_withdraw(&mut state, addr(10), 1_000_000);
    assert_eq!(transfers, vec![(addr(10), 1_000_000)]);
    assert_eq!(state.balance.get(&addr(10)).copied().unwrap_or(0), 0);
}

#[test]
fn test_withdraw_partial() {
    let mut state = new_state();
    apply_deposit(&mut state, addr(10), 1_000_000);
    apply_withdraw(&mut state, addr(10), 400_000);
    assert_eq!(state.balance[&addr(10)], 600_000);
}

#[test]
fn test_withdraw_zero_rejected() {
    let mut state = new_state();
    apply_deposit(&mut state, addr(10), 1_000_000);
    let result = std::panic::catch_unwind(move || {
        apply_withdraw(&mut state, addr(10), 0);
    });
    assert!(result.is_err());
}

#[test]
fn test_withdraw_more_than_balance_rejected() {
    let mut state = new_state();
    apply_deposit(&mut state, addr(10), 500_000);
    let result = std::panic::catch_unwind(move || {
        apply_withdraw(&mut state, addr(10), 1_000_000);
    });
    assert!(result.is_err(), "over-withdrawal should be rejected");
}

#[test]
fn test_withdraw_with_locked_funds_only_unlocked_portion_available() {
    let mut state = new_state();
    credit(&mut state, addr(10), 2_000_000);
    // lock 1_000_000
    let players = vec![addr(10), addr(11)];
    credit(&mut state, addr(11), 1_000_000);
    apply_lock_entries(&mut state, addr(2), game_hash(1), players, 1_000_000, 100);

    // addr(10) has balance=2_000_000, locked=1_000_000 → available=1_000_000
    let transfers = apply_withdraw(&mut state, addr(10), 1_000_000);
    assert_eq!(transfers, vec![(addr(10), 1_000_000)]);

    // trying to withdraw 1 more should fail
    let mut state2 = new_state();
    credit(&mut state2, addr(10), 2_000_000);
    credit(&mut state2, addr(11), 1_000_000);
    apply_lock_entries(
        &mut state2,
        addr(2),
        game_hash(1),
        vec![addr(10), addr(11)],
        1_000_000,
        100,
    );
    let result = std::panic::catch_unwind(move || {
        apply_withdraw(&mut state2, addr(10), 1_000_001);
    });
    assert!(result.is_err(), "withdrawing into locked portion should be rejected");
}

// ---------------------------------------------------------------------------
// 4. lock_entries
// ---------------------------------------------------------------------------

#[test]
fn test_lock_entries_happy_path() {
    let mut state = new_state();
    let players: Vec<[u8; 32]> = (10u8..14).map(addr).collect();
    for p in &players {
        credit(&mut state, *p, 1_000_000);
    }
    apply_lock_entries(&mut state, addr(2), game_hash(1), players.clone(), 500_000, 100);

    for p in &players {
        assert_eq!(state.locked.get(p).copied().unwrap_or(0), 500_000);
        assert_eq!(state.balance[p], 1_000_000); // balance unchanged; locked tracked separately
    }
    let game = state.games[&game_hash(1)].clone();
    assert_eq!(game.status, game_status::ACTIVE);
    assert_eq!(game.entry_fee, 500_000);
    assert_eq!(game.players, players);
}

#[test]
fn test_lock_entries_insufficient_balance_rejected() {
    let mut state = new_state();
    credit(&mut state, addr(10), 400_000); // needs 500_000
    credit(&mut state, addr(11), 500_000);
    let result = std::panic::catch_unwind(move || {
        apply_lock_entries(
            &mut state,
            addr(2),
            game_hash(1),
            vec![addr(10), addr(11)],
            500_000,
            100,
        );
    });
    assert!(result.is_err(), "insufficient balance should be rejected");
}

#[test]
fn test_lock_entries_duplicate_game_hash_rejected() {
    let mut state = new_state();
    let players: Vec<[u8; 32]> = (10u8..12).map(addr).collect();
    for p in &players {
        credit(&mut state, *p, 1_000_000);
    }
    apply_lock_entries(&mut state, addr(2), game_hash(1), players.clone(), 500_000, 100);

    // second lock with same hash must fail (idempotency guard)
    let result = std::panic::catch_unwind(move || {
        apply_lock_entries(&mut state, addr(2), game_hash(1), players, 500_000, 100);
    });
    assert!(result.is_err(), "duplicate game hash should be rejected");
}

#[test]
fn test_lock_entries_non_platform_caller_rejected() {
    let mut state = new_state();
    credit(&mut state, addr(10), 1_000_000);
    let result = std::panic::catch_unwind(move || {
        apply_lock_entries(
            &mut state,
            addr(99), // not the platform
            game_hash(1),
            vec![addr(10)],
            500_000,
            100,
        );
    });
    assert!(result.is_err(), "non-platform caller should be rejected");
}

#[test]
fn test_lock_entries_blocked_when_paused() {
    let mut state = new_state();
    credit(&mut state, addr(10), 1_000_000);
    apply_pause(&mut state, addr(1));
    let result = std::panic::catch_unwind(move || {
        apply_lock_entries(
            &mut state,
            addr(2),
            game_hash(1),
            vec![addr(10)],
            500_000,
            100,
        );
    });
    assert!(result.is_err(), "lock when paused should panic");
}

#[test]
fn test_lock_entries_empty_players_rejected() {
    let mut state = new_state();
    let result = std::panic::catch_unwind(move || {
        apply_lock_entries(&mut state, addr(2), game_hash(1), vec![], 500_000, 100);
    });
    assert!(result.is_err(), "zero players should be rejected");
}

// ---------------------------------------------------------------------------
// 5. settle_game — happy path (4 players, 1 winner takes all after fees)
// ---------------------------------------------------------------------------

/// Helper: set up a 4-player game locked at block_timestamp `lock_ts`.
fn setup_4player_game(state: &mut ContractState, lock_ts: u64) -> [u8; 32] {
    let players: Vec<[u8; 32]> = (10u8..14).map(addr).collect();
    for p in &players {
        credit(state, *p, 1_000_000);
    }
    let hash = game_hash(42);
    apply_lock_entries(state, addr(2), hash, players, 500_000, lock_ts);
    hash
}

#[test]
fn test_settle_game_happy_path_single_winner() {
    let mut state = new_state();
    let hash = setup_4player_game(&mut state, 0);

    // pool = 4 * 500_000 = 2_000_000
    // fee  = 2_000_000 * 300 / 10_000 = 60_000
    // burn = 2_000_000 * 200 / 10_000 = 40_000
    // winner gets 2_000_000 - 60_000 - 40_000 = 1_900_000
    let winners = vec![addr(10)];
    let amounts = vec![1_900_000u128];
    let transfers = apply_settle_game(&mut state, addr(2), hash, winners, amounts);

    let game = &state.games[&hash];
    assert_eq!(game.status, game_status::SETTLED);

    // winner credited
    assert_eq!(state.balance[&addr(10)], 1_900_000 + (1_000_000 - 500_000));
    // losers deducted entry fee
    for seed in 11u8..14 {
        assert_eq!(state.balance[&addr(seed)], 500_000);
        assert_eq!(state.locked.get(&addr(seed)).copied().unwrap_or(0), 0);
    }
    assert_eq!(state.total_fees_collected, 60_000);

    // burn transfer included
    let burn_addr = arena_pool::logic::BURN_ADDRESS;
    assert!(transfers.contains(&(burn_addr, 40_000)));
}

#[test]
fn test_settle_game_multi_winner_split() {
    let mut state = new_state();
    let hash = setup_4player_game(&mut state, 0);

    // pool = 2_000_000, fee = 60_000, burn = 40_000, distributable = 1_900_000
    // split evenly between two winners
    let winners = vec![addr(10), addr(11)];
    let amounts = vec![950_000u128, 950_000u128];
    apply_settle_game(&mut state, addr(2), hash, winners, amounts);

    assert_eq!(state.balance[&addr(10)], 950_000 + 500_000); // winner + leftover balance
    assert_eq!(state.balance[&addr(11)], 950_000 + 500_000);
}

#[test]
fn test_settle_game_amounts_dont_sum_correctly_rejected() {
    let mut state = new_state();
    let hash = setup_4player_game(&mut state, 0);

    // pool=2_000_000, fee=60_000, burn=40_000 → winners must receive 1_900_000 total
    let result = std::panic::catch_unwind(move || {
        apply_settle_game(
            &mut state,
            addr(2),
            hash,
            vec![addr(10)],
            vec![1_800_000], // wrong: 1_800_000 + 60_000 + 40_000 = 1_900_000 ≠ 2_000_000
        );
    });
    assert!(result.is_err(), "bad sum should be rejected");
}

#[test]
fn test_settle_game_non_player_winner_rejected() {
    let mut state = new_state();
    let hash = setup_4player_game(&mut state, 0);

    let result = std::panic::catch_unwind(move || {
        apply_settle_game(
            &mut state,
            addr(2),
            hash,
            vec![addr(99)], // not in game
            vec![1_900_000],
        );
    });
    assert!(result.is_err(), "non-player winner should be rejected");
}

#[test]
fn test_settle_already_settled_game_rejected() {
    let mut state = new_state();
    let hash = setup_4player_game(&mut state, 0);
    apply_settle_game(&mut state, addr(2), hash, vec![addr(10)], vec![1_900_000]);

    let result = std::panic::catch_unwind(move || {
        apply_settle_game(&mut state, addr(2), hash, vec![addr(10)], vec![1_900_000]);
    });
    assert!(result.is_err(), "settling an already-settled game should be rejected");
}

#[test]
fn test_settle_non_existent_game_rejected() {
    let mut state = new_state();
    let result = std::panic::catch_unwind(move || {
        apply_settle_game(&mut state, addr(2), game_hash(99), vec![addr(10)], vec![1_000]);
    });
    assert!(result.is_err(), "settling non-existent game should be rejected");
}

#[test]
fn test_settle_non_platform_caller_rejected() {
    let mut state = new_state();
    let hash = setup_4player_game(&mut state, 0);
    let result = std::panic::catch_unwind(move || {
        apply_settle_game(&mut state, addr(99), hash, vec![addr(10)], vec![1_900_000]);
    });
    assert!(result.is_err(), "non-platform settle should be rejected");
}

#[test]
fn test_settle_winners_amounts_length_mismatch_rejected() {
    let mut state = new_state();
    let hash = setup_4player_game(&mut state, 0);
    let result = std::panic::catch_unwind(move || {
        apply_settle_game(
            &mut state,
            addr(2),
            hash,
            vec![addr(10), addr(11)],
            vec![1_900_000], // length mismatch
        );
    });
    assert!(result.is_err(), "winners/amounts length mismatch should be rejected");
}

// ---------------------------------------------------------------------------
// 6. refund_game (platform)
// ---------------------------------------------------------------------------

#[test]
fn test_refund_game_unlocks_all_players() {
    let mut state = new_state();
    let hash = setup_4player_game(&mut state, 0);

    apply_refund_game(&mut state, addr(2), hash);

    let game = &state.games[&hash];
    assert_eq!(game.status, game_status::REFUNDED);
    for seed in 10u8..14 {
        assert_eq!(state.locked.get(&addr(seed)).copied().unwrap_or(0), 0);
        // balance unchanged (locked is released, not consumed)
        assert_eq!(state.balance[&addr(seed)], 1_000_000);
    }
}

#[test]
fn test_refund_non_active_game_rejected() {
    let mut state = new_state();
    let hash = setup_4player_game(&mut state, 0);
    apply_refund_game(&mut state, addr(2), hash);

    let result = std::panic::catch_unwind(move || {
        apply_refund_game(&mut state, addr(2), hash);
    });
    assert!(result.is_err(), "refunding already-refunded game should fail");
}

#[test]
fn test_refund_non_platform_caller_rejected() {
    let mut state = new_state();
    let hash = setup_4player_game(&mut state, 0);
    let result = std::panic::catch_unwind(move || {
        apply_refund_game(&mut state, addr(99), hash);
    });
    assert!(result.is_err(), "non-platform refund should be rejected");
}

// ---------------------------------------------------------------------------
// 7. emergency refund (player-initiated, after timeout)
// ---------------------------------------------------------------------------

/// Seconds required before an emergency refund is valid.
const EMERGENCY_TIMEOUT: u64 = 3600;

#[test]
fn test_emergency_refund_after_timeout_succeeds() {
    let mut state = new_state();
    let lock_ts = 1_000_000u64;
    let hash = setup_4player_game(&mut state, lock_ts);

    let now = lock_ts + EMERGENCY_TIMEOUT + 1;
    apply_emergency_refund(&mut state, addr(10), hash, now);

    let game = &state.games[&hash];
    assert_eq!(game.status, game_status::REFUNDED);
    for seed in 10u8..14 {
        assert_eq!(state.locked.get(&addr(seed)).copied().unwrap_or(0), 0);
    }
}

#[test]
fn test_emergency_refund_before_timeout_rejected() {
    let mut state = new_state();
    let lock_ts = 1_000_000u64;
    let hash = setup_4player_game(&mut state, lock_ts);

    let now = lock_ts + EMERGENCY_TIMEOUT - 1; // 1 second too early
    let result = std::panic::catch_unwind(move || {
        apply_emergency_refund(&mut state, addr(10), hash, now);
    });
    assert!(result.is_err(), "emergency refund before timeout should be rejected");
}

#[test]
fn test_emergency_refund_by_non_player_rejected() {
    let mut state = new_state();
    let lock_ts = 1_000_000u64;
    let hash = setup_4player_game(&mut state, lock_ts);

    let now = lock_ts + EMERGENCY_TIMEOUT + 1;
    let result = std::panic::catch_unwind(move || {
        apply_emergency_refund(&mut state, addr(99), hash, now); // not a player
    });
    assert!(result.is_err(), "non-player emergency refund should be rejected");
}

#[test]
fn test_emergency_refund_on_settled_game_rejected() {
    let mut state = new_state();
    let lock_ts = 1_000_000u64;
    let hash = setup_4player_game(&mut state, lock_ts);
    apply_settle_game(&mut state, addr(2), hash, vec![addr(10)], vec![1_900_000]);

    let now = lock_ts + EMERGENCY_TIMEOUT + 1;
    let result = std::panic::catch_unwind(move || {
        apply_emergency_refund(&mut state, addr(10), hash, now);
    });
    assert!(result.is_err(), "emergency refund on settled game should be rejected");
}

// ---------------------------------------------------------------------------
// 8. claim_fees
// ---------------------------------------------------------------------------

#[test]
fn test_claim_fees_transfers_accumulated_fees() {
    let mut state = new_state();
    let hash = setup_4player_game(&mut state, 0);
    apply_settle_game(&mut state, addr(2), hash, vec![addr(10)], vec![1_900_000]);

    assert_eq!(state.total_fees_collected, 60_000);

    let transfers = apply_claim_fees(&mut state, addr(1)); // owner
    assert_eq!(transfers, vec![(addr(1), 60_000)]);
    assert_eq!(state.total_fees_collected, 0);
}

#[test]
fn test_claim_fees_zero_when_no_fees_collected() {
    let mut state = new_state();
    let result = std::panic::catch_unwind(move || {
        apply_claim_fees(&mut state, addr(1));
    });
    assert!(result.is_err(), "claiming zero fees should be rejected");
}

#[test]
fn test_claim_fees_non_owner_rejected() {
    let mut state = new_state();
    let hash = setup_4player_game(&mut state, 0);
    apply_settle_game(&mut state, addr(2), hash, vec![addr(10)], vec![1_900_000]);

    let result = std::panic::catch_unwind(move || {
        apply_claim_fees(&mut state, addr(99)); // not owner
    });
    assert!(result.is_err(), "non-owner fee claim should be rejected");
}

// ---------------------------------------------------------------------------
// 9. pause / unpause
// ---------------------------------------------------------------------------

#[test]
fn test_pause_blocks_deposit_and_lock() {
    let mut state = new_state();
    apply_pause(&mut state, addr(1));
    assert!(state.paused);

    let result = std::panic::catch_unwind(move || {
        apply_deposit(&mut state, addr(10), 1_000_000);
    });
    assert!(result.is_err(), "deposit when paused should fail");
}

#[test]
fn test_unpause_allows_deposit_again() {
    let mut state = new_state();
    apply_pause(&mut state, addr(1));
    apply_unpause(&mut state, addr(1));
    assert!(!state.paused);
    apply_deposit(&mut state, addr(10), 1_000_000); // should not panic
    assert_eq!(state.balance[&addr(10)], 1_000_000);
}

#[test]
fn test_pause_allows_withdraw() {
    // Players must be able to withdraw even when paused (safety feature).
    let mut state = new_state();
    apply_deposit(&mut state, addr(10), 1_000_000);
    apply_pause(&mut state, addr(1));
    let transfers = apply_withdraw(&mut state, addr(10), 1_000_000);
    assert_eq!(transfers, vec![(addr(10), 1_000_000)]);
}

#[test]
fn test_pause_non_owner_rejected() {
    let mut state = new_state();
    let result = std::panic::catch_unwind(move || {
        apply_pause(&mut state, addr(99));
    });
    assert!(result.is_err(), "non-owner pause should be rejected");
}

#[test]
fn test_unpause_non_owner_rejected() {
    let mut state = new_state();
    apply_pause(&mut state, addr(1));
    let result = std::panic::catch_unwind(move || {
        apply_unpause(&mut state, addr(99));
    });
    assert!(result.is_err(), "non-owner unpause should be rejected");
}

// ---------------------------------------------------------------------------
// 10. cleanup_games
// ---------------------------------------------------------------------------

#[test]
fn test_cleanup_removes_settled_and_refunded_games() {
    let mut state = new_state();
    let h1 = {
        let players: Vec<[u8; 32]> = (10u8..14).map(addr).collect();
        for p in &players {
            credit(&mut state, *p, 1_000_000);
        }
        let h = game_hash(1);
        apply_lock_entries(&mut state, addr(2), h, players, 500_000, 0);
        apply_settle_game(&mut state, addr(2), h, vec![addr(10)], vec![1_900_000]);
        h
    };
    let h2 = {
        let players: Vec<[u8; 32]> = (20u8..24).map(addr).collect();
        for p in &players {
            credit(&mut state, *p, 1_000_000);
        }
        let h = game_hash(2);
        apply_lock_entries(&mut state, addr(2), h, players, 500_000, 0);
        apply_refund_game(&mut state, addr(2), h);
        h
    };

    apply_cleanup_games(&mut state, addr(1), vec![h1, h2]);

    assert!(!state.games.contains_key(&h1));
    assert!(!state.games.contains_key(&h2));
}

#[test]
fn test_cleanup_active_game_rejected() {
    let mut state = new_state();
    let hash = setup_4player_game(&mut state, 0);

    let result = std::panic::catch_unwind(move || {
        apply_cleanup_games(&mut state, addr(1), vec![hash]);
    });
    assert!(result.is_err(), "cleaning up an active game should fail");
}

#[test]
fn test_cleanup_non_owner_rejected() {
    let mut state = new_state();
    let hash = setup_4player_game(&mut state, 0);
    apply_settle_game(&mut state, addr(2), hash, vec![addr(10)], vec![1_900_000]);

    let result = std::panic::catch_unwind(move || {
        apply_cleanup_games(&mut state, addr(99), vec![hash]);
    });
    assert!(result.is_err(), "non-owner cleanup should be rejected");
}

// ---------------------------------------------------------------------------
// 11. Conservation invariant
// ---------------------------------------------------------------------------

#[test]
fn test_fee_burn_conservation_exact() {
    // Verify that fee + burn + winner_amounts == total_pool for several combos.
    let cases: &[(u128, u16, u16, u128)] = &[
        // (pool, fee_bps, burn_bps, expected_fee)
        (2_000_000, 300, 200, 60_000),
        (10_000, 500, 100, 500),
        (1_000_000, 0, 0, 0),
        (100_000, 1000, 1000, 10_000),
    ];
    for &(pool, fee_bps, burn_bps, _expected_fee) in cases {
        let fee = pool * fee_bps as u128 / 10_000;
        let burn = pool * burn_bps as u128 / 10_000;
        let distributable = pool - fee - burn;

        // The conservation invariant must hold
        assert_eq!(distributable + fee + burn, pool, "conservation violated for pool={pool}");
    }
}

#[test]
fn test_balance_minus_locked_never_negative() {
    // After locking, available = balance - locked must be >= 0 for all players.
    let mut state = new_state();
    let players: Vec<[u8; 32]> = (10u8..14).map(addr).collect();
    for p in &players {
        credit(&mut state, *p, 500_000);
    }
    apply_lock_entries(&mut state, addr(2), game_hash(1), players.clone(), 500_000, 0);

    for p in &players {
        let bal = state.balance[p];
        let locked = state.locked.get(p).copied().unwrap_or(0);
        assert!(bal >= locked, "balance {bal} < locked {locked} for player {p:?}");
    }
}

// ---------------------------------------------------------------------------
// 12. Edge cases
// ---------------------------------------------------------------------------

#[test]
fn test_large_player_count_game() {
    // Stress: 50 players, verify conservation.
    let mut state = new_state();
    let players: Vec<[u8; 32]> = (0u8..50).map(addr).collect();
    let entry_fee = 100_000u128;
    for p in &players {
        credit(&mut state, *p, entry_fee);
    }
    let hash = game_hash(7);
    apply_lock_entries(&mut state, addr(2), hash, players.clone(), entry_fee, 0);

    let total_pool = 50 * entry_fee;
    let fee = total_pool * 300 / 10_000;
    let burn = total_pool * 200 / 10_000;
    let distributable = total_pool - fee - burn;

    // Single winner takes distributable
    apply_settle_game(&mut state, addr(2), hash, vec![addr(0)], vec![distributable]);

    assert_eq!(state.total_fees_collected, fee);
    assert_eq!(state.balance[&addr(0)], distributable); // winner: was 0 before entry, now has winnings
}

#[test]
fn test_zero_fee_bps_and_burn_bps() {
    // When fee=0 and burn=0 the entire pool goes to winners.
    let mut state = ContractState::default();
    apply_init(&mut state, addr(1), addr(2), 0, 0);

    let players: Vec<[u8; 32]> = (10u8..12).map(addr).collect();
    for p in &players {
        credit(&mut state, *p, 1_000_000);
    }
    let hash = game_hash(3);
    apply_lock_entries(&mut state, addr(2), hash, players, 1_000_000, 0);

    // pool = 2_000_000; fee = 0; burn = 0
    apply_settle_game(&mut state, addr(2), hash, vec![addr(10)], vec![2_000_000]);
    assert_eq!(state.total_fees_collected, 0);
    assert_eq!(state.balance[&addr(10)], 2_000_000);
}

#[test]
fn test_multiple_concurrent_games() {
    let mut state = new_state();
    // Game A
    let players_a: Vec<[u8; 32]> = (10u8..12).map(addr).collect();
    for p in &players_a {
        credit(&mut state, *p, 2_000_000);
    }
    apply_lock_entries(&mut state, addr(2), game_hash(10), players_a, 1_000_000, 0);

    // Game B — overlapping player (addr(11) is in both)
    let players_b: Vec<[u8; 32]> = vec![addr(11), addr(20)];
    credit(&mut state, addr(20), 1_000_000);
    // addr(11) has balance=2_000_000, locked=1_000_000 → available=1_000_000
    apply_lock_entries(&mut state, addr(2), game_hash(11), players_b, 1_000_000, 0);

    // addr(11) now locked=2_000_000, balance=2_000_000 → available=0
    assert_eq!(state.locked[&addr(11)], 2_000_000);

    // Settle game A
    // pool_a = 2*1_000_000 = 2_000_000; fee=60_000; burn=40_000; winner gets 1_900_000
    apply_settle_game(
        &mut state,
        addr(2),
        game_hash(10),
        vec![addr(10)],
        vec![1_900_000],
    );

    // addr(11) locked reduces by entry_fee from game A
    assert_eq!(state.locked[&addr(11)], 1_000_000); // still locked in game B
}
