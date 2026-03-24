//! Pure business logic for the Arena Pool contract.
//!
//! All functions operate on [`ContractState`] and return structured results.
//! No host-function calls are made here — those are handled by the Wasm entry
//! points in `lib.rs`.  This design makes every invariant testable natively.

use std::collections::BTreeMap;
use borsh::{BorshDeserialize, BorshSerialize};

use crate::types::{game_status, GameInfo};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Address to which burn amounts are sent (all-zeros = provably unspendable).
pub const BURN_ADDRESS: [u8; 32] = [0u8; 32];

/// Seconds a game must be locked before an emergency refund is allowed.
pub const EMERGENCY_TIMEOUT_SECS: u64 = 3_600;

// ---------------------------------------------------------------------------
// Contract state
// ---------------------------------------------------------------------------

/// Full in-memory state of the contract.
///
/// Serialised as a single borsh blob under the `"__state__"` storage key.
/// Using a single blob keeps state snapshots atomic and avoids partial-write
/// issues that arise with per-key storage in the test harness.
#[derive(BorshSerialize, BorshDeserialize, Default, Clone, Debug)]
pub struct ContractState {
    pub version: u32,
    pub paused: bool,
    pub owner: [u8; 32],
    pub platform: [u8; 32],
    /// Platform fee in basis points (max 10 000)
    pub fee_bps: u16,
    /// Burn fraction in basis points (max 10 000)
    pub burn_bps: u16,
    /// `balance[addr]` — total deposited (including locked portion)
    pub balance: BTreeMap<[u8; 32], u128>,
    /// `locked[addr]` — portion of balance currently locked in active games
    pub locked: BTreeMap<[u8; 32], u128>,
    /// Active, settled, and refunded games
    pub games: BTreeMap<[u8; 32], GameInfo>,
    /// Accumulated platform fees (claimable by owner)
    pub total_fees_collected: u128,
}

impl ContractState {
    /// Available (unlocked) balance for a player.
    pub fn available(&self, addr: &[u8; 32]) -> u128 {
        let bal = self.balance.get(addr).copied().unwrap_or(0);
        let lck = self.locked.get(addr).copied().unwrap_or(0);
        bal.saturating_sub(lck)
    }
}

// ---------------------------------------------------------------------------
// Helpers — internal assertions use panic so tests can catch_unwind them
// ---------------------------------------------------------------------------

#[inline(always)]
fn require(cond: bool, msg: &str) {
    if !cond {
        panic!("{}", msg);
    }
}

// ---------------------------------------------------------------------------
// 1. init
// ---------------------------------------------------------------------------

pub fn apply_init(
    state: &mut ContractState,
    owner: [u8; 32],
    platform: [u8; 32],
    fee_bps: u16,
    burn_bps: u16,
) {
    require(
        (fee_bps as u32) + (burn_bps as u32) <= 10_000,
        "fee_bps + burn_bps must be <= 10000",
    );
    state.version = 1;
    state.paused = false;
    state.owner = owner;
    state.platform = platform;
    state.fee_bps = fee_bps;
    state.burn_bps = burn_bps;
    state.total_fees_collected = 0;
}

// ---------------------------------------------------------------------------
// 2. deposit
// ---------------------------------------------------------------------------

pub fn apply_deposit(state: &mut ContractState, caller: [u8; 32], value: u128) {
    require(!state.paused, "contract is paused");
    require(value > 0, "deposit value must be > 0");
    *state.balance.entry(caller).or_insert(0) += value;
}

// ---------------------------------------------------------------------------
// 3. withdraw  (Checks-Effects-Interactions)
// ---------------------------------------------------------------------------

/// Returns a list of `(recipient, amount)` transfers to execute.
pub fn apply_withdraw(
    state: &mut ContractState,
    caller: [u8; 32],
    amount: u128,
) -> Vec<([u8; 32], u128)> {
    require(amount > 0, "withdraw amount must be > 0");
    let available = state.available(&caller);
    require(available >= amount, "insufficient available balance");

    // CHECKS-EFFECTS: deduct balance BEFORE any transfer
    *state.balance.entry(caller).or_insert(0) -= amount;

    // INTERACTION: return transfer instruction
    vec![(caller, amount)]
}

// ---------------------------------------------------------------------------
// 4. lock_entries
// ---------------------------------------------------------------------------

pub fn apply_lock_entries(
    state: &mut ContractState,
    caller: [u8; 32],
    game_hash: [u8; 32],
    players: Vec<[u8; 32]>,
    entry_fee: u128,
    lock_time: u64,
) {
    require(!state.paused, "contract is paused");
    require(caller == state.platform, "caller is not the platform");
    require(!players.is_empty(), "players list must not be empty");
    require(entry_fee > 0, "entry_fee must be > 0");
    require(
        !state.games.contains_key(&game_hash),
        "game already exists (idempotency violation)",
    );

    // Verify each player has sufficient unlocked balance
    for p in &players {
        let avail = state.available(p);
        require(
            avail >= entry_fee,
            "player has insufficient available balance",
        );
    }

    // Lock entry fee for each player
    for p in &players {
        *state.locked.entry(*p).or_insert(0) += entry_fee;
    }

    state.games.insert(
        game_hash,
        GameInfo {
            status: game_status::ACTIVE,
            entry_fee,
            lock_time,
            players,
        },
    );
}

// ---------------------------------------------------------------------------
// 5. settle_game
// ---------------------------------------------------------------------------

/// Returns a list of `(recipient, amount)` transfers to execute.
/// Includes the burn transfer to BURN_ADDRESS.
pub fn apply_settle_game(
    state: &mut ContractState,
    caller: [u8; 32],
    game_hash: [u8; 32],
    winners: Vec<[u8; 32]>,
    amounts: Vec<u128>,
) -> Vec<([u8; 32], u128)> {
    require(caller == state.platform, "caller is not the platform");
    require(winners.len() == amounts.len(), "winners and amounts length mismatch");

    let game = state
        .games
        .get(&game_hash)
        .expect("game does not exist")
        .clone();

    require(game.status == game_status::ACTIVE, "game is not active");

    // Verify all winners are registered players
    for w in &winners {
        require(
            game.players.contains(w),
            "winner is not a participant in this game",
        );
    }

    let player_count = game.players.len() as u128;
    let total_pool = player_count * game.entry_fee;
    let platform_fee = total_pool * (state.fee_bps as u128) / 10_000;
    let burn_amount = total_pool * (state.burn_bps as u128) / 10_000;
    let winner_total: u128 = amounts.iter().sum();

    require(
        winner_total + platform_fee + burn_amount == total_pool,
        "amounts + fee + burn must equal total_pool (conservation violation)",
    );

    // EFFECTS — deduct entry fee from every player (balance + locked)
    for p in &game.players {
        *state.balance.entry(*p).or_insert(0) -= game.entry_fee;
        *state.locked.entry(*p).or_insert(0) -= game.entry_fee;
    }

    // Credit winners
    for (w, &amt) in winners.iter().zip(amounts.iter()) {
        *state.balance.entry(*w).or_insert(0) += amt;
    }

    // Accumulate platform fee (held in contract, claimable later)
    state.total_fees_collected += platform_fee;

    // Mark settled
    state.games.get_mut(&game_hash).expect("game disappeared after validation: state corruption").status = game_status::SETTLED;

    // INTERACTION — return transfer instructions
    let mut transfers = Vec::new();
    if burn_amount > 0 {
        transfers.push((BURN_ADDRESS, burn_amount));
    }
    transfers
}

// ---------------------------------------------------------------------------
// 6. refund_game (platform)
// ---------------------------------------------------------------------------

pub fn apply_refund_game(state: &mut ContractState, caller: [u8; 32], game_hash: [u8; 32]) {
    require(caller == state.platform, "caller is not the platform");

    let game = state
        .games
        .get(&game_hash)
        .expect("game does not exist")
        .clone();

    require(game.status == game_status::ACTIVE, "game is not active");

    // Unlock all players
    for p in &game.players {
        *state.locked.entry(*p).or_insert(0) -= game.entry_fee;
    }

    state.games.get_mut(&game_hash).expect("game disappeared after validation: state corruption").status = game_status::REFUNDED;
}

// ---------------------------------------------------------------------------
// 7. emergency refund (player-initiated, after timeout)
// ---------------------------------------------------------------------------

pub fn apply_emergency_refund(
    state: &mut ContractState,
    caller: [u8; 32],
    game_hash: [u8; 32],
    now: u64,
) {
    let game = state
        .games
        .get(&game_hash)
        .expect("game does not exist")
        .clone();

    require(game.status == game_status::ACTIVE, "game is not active");

    // Only a participant may trigger the emergency refund
    require(
        game.players.contains(&caller),
        "caller is not a participant in this game",
    );

    require(
        now >= game.lock_time + EMERGENCY_TIMEOUT_SECS,
        "emergency timeout has not elapsed",
    );

    // Unlock all players
    for p in &game.players {
        *state.locked.entry(*p).or_insert(0) -= game.entry_fee;
    }

    state.games.get_mut(&game_hash).expect("game disappeared after validation: state corruption").status = game_status::REFUNDED;
}

// ---------------------------------------------------------------------------
// 8. claim_fees
// ---------------------------------------------------------------------------

/// Returns the transfer to send accumulated fees to the owner.
pub fn apply_claim_fees(state: &mut ContractState, caller: [u8; 32]) -> Vec<([u8; 32], u128)> {
    require(caller == state.owner, "caller is not the owner");
    require(state.total_fees_collected > 0, "no fees to claim");

    // CEI: capture amount, zero out storage, then return transfer instruction
    let amount = state.total_fees_collected;
    state.total_fees_collected = 0;

    vec![(state.owner, amount)]
}

// ---------------------------------------------------------------------------
// 9. pause / unpause
// ---------------------------------------------------------------------------

pub fn apply_pause(state: &mut ContractState, caller: [u8; 32]) {
    require(caller == state.owner, "caller is not the owner");
    require(!state.paused, "contract is already paused");
    state.paused = true;
}

pub fn apply_unpause(state: &mut ContractState, caller: [u8; 32]) {
    require(caller == state.owner, "caller is not the owner");
    require(state.paused, "contract is not paused");
    state.paused = false;
}

// ---------------------------------------------------------------------------
// 10. cleanup_games
// ---------------------------------------------------------------------------

pub fn apply_cleanup_games(
    state: &mut ContractState,
    caller: [u8; 32],
    hashes: Vec<[u8; 32]>,
) {
    require(caller == state.owner, "caller is not the owner");

    for hash in &hashes {
        let game = state.games.get(hash).expect("game does not exist");
        require(
            game.status != game_status::ACTIVE,
            "cannot delete an active game",
        );
    }

    for hash in &hashes {
        state.games.remove(hash);
    }
}
