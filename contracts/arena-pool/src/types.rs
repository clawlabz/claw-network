//! Domain types for the Arena Pool contract.

use borsh::{BorshDeserialize, BorshSerialize};

/// Status codes for a game's lifecycle.
pub mod game_status {
    pub const ACTIVE: u8 = 0;
    pub const SETTLED: u8 = 1;
    pub const REFUNDED: u8 = 2;
}

/// All information stored per game in the contract.
#[derive(BorshSerialize, BorshDeserialize, Clone, Debug, PartialEq)]
pub struct GameInfo {
    /// 0 = Active, 1 = Settled, 2 = Refunded
    pub status: u8,
    /// Entry fee per player (in attoCAW / smallest unit)
    pub entry_fee: u128,
    /// block_timestamp at the time entries were locked
    pub lock_time: u64,
    /// List of participating player addresses
    pub players: Vec<[u8; 32]>,
}

/// Arguments for `init`.
#[derive(BorshSerialize, BorshDeserialize)]
pub struct InitArgs {
    pub owner: [u8; 32],
    pub platform: [u8; 32],
    /// Platform fee in basis points (e.g. 300 = 3 %)
    pub fee_bps: u16,
    /// Burn amount in basis points (e.g. 200 = 2 %)
    pub burn_bps: u16,
}

/// Arguments for `deposit` (no fields — value comes from msg.value).
#[derive(BorshSerialize, BorshDeserialize)]
pub struct DepositArgs {}

/// Arguments for `withdraw`.
#[derive(BorshSerialize, BorshDeserialize)]
pub struct WithdrawArgs {
    pub amount: u128,
}

/// Arguments for `lock_entries`.
#[derive(BorshSerialize, BorshDeserialize)]
pub struct LockEntriesArgs {
    pub game_hash: [u8; 32],
    pub players: Vec<[u8; 32]>,
    pub entry_fee: u128,
}

/// Arguments for `settle_game`.
#[derive(BorshSerialize, BorshDeserialize)]
pub struct SettleGameArgs {
    pub game_hash: [u8; 32],
    /// Addresses of winners (must be a subset of game.players)
    pub winners: Vec<[u8; 32]>,
    /// Payout amounts, parallel to `winners`
    pub amounts: Vec<u128>,
}

/// Arguments for `refund_game` (platform-initiated).
#[derive(BorshSerialize, BorshDeserialize)]
pub struct RefundGameArgs {
    pub game_hash: [u8; 32],
}

/// Arguments for `refund_game_emergency` (player-initiated, after timeout).
#[derive(BorshSerialize, BorshDeserialize)]
pub struct EmergencyRefundArgs {
    pub game_hash: [u8; 32],
}

/// Arguments for `claim_fees`.
#[derive(BorshSerialize, BorshDeserialize)]
pub struct ClaimFeesArgs {}

/// Arguments for `pause` / `unpause`.
#[derive(BorshSerialize, BorshDeserialize)]
pub struct PauseArgs {}

/// Arguments for `cleanup_games`.
#[derive(BorshSerialize, BorshDeserialize)]
pub struct CleanupGamesArgs {
    pub hashes: Vec<[u8; 32]>,
}
