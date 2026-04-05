//! Block reward distribution for ClawNetwork.
//!
//! Rewards are funded from the Node Incentive Pool (genesis address index 1)
//! and follow a decay schedule over 10+ years.

use claw_types::block::BlockEvent;
use claw_types::state::*;
use crate::WorldState;

/// Blocks per year assuming 3-second block time: 365.25 * 86400 / 3.
pub const BLOCKS_PER_YEAR: u64 = 10_512_000;

/// Genesis address index for the Node Incentive Pool (40% of total supply).
pub const NODE_INCENTIVE_POOL_INDEX: u8 = 1;

/// Genesis address index for the Ecosystem Fund (25% of total supply).
pub const ECOSYSTEM_FUND_INDEX: u8 = 2;

/// Block height at which the mining upgrade activates.
/// Before this height, validators get 100% of block rewards (legacy schedule).
/// After this height, rewards split: 65% validators, 35% miners.
pub const MINING_UPGRADE_HEIGHT: u64 = 2_000;

/// Validator share of block rewards after mining upgrade, in basis points.
pub const VALIDATOR_REWARD_BPS: u128 = 6_500;

/// Mining share of block rewards after mining upgrade, in basis points.
pub const MINING_REWARD_BPS: u128 = 3_500;

/// Halving period for the new reward schedule (2 years of blocks).
pub const HALVING_PERIOD: u64 = 2 * BLOCKS_PER_YEAR; // 21_024_000

/// Derive a genesis address from its allocation index.
/// Must match the derivation in `crate::genesis`.
fn genesis_address(index: u8) -> [u8; 32] {
    let mut addr = [0u8; 32];
    addr[0] = index;
    addr
}

/// Public version of genesis_address for use in tests.
pub fn genesis_address_pub(index: u8) -> [u8; 32] {
    genesis_address(index)
}

/// Calculate the block reward (in base units, 9 decimals) for a given height.
///
/// **Legacy schedule** (before MINING_UPGRADE_HEIGHT):
/// - Year 1: 10 CLAW, Year 2: 8, Year 3: 6, Year 4: 4, Year 5-10: 2, Year 11+: 1
///
/// **New schedule** (after MINING_UPGRADE_HEIGHT):
/// Geometric halving every 2 years starting from 8 CLAW:
/// - Period 0: 8 CLAW, Period 1: 4, Period 2: 2, Period 3: 1, Period 4: 0.5, Period 5+: 0.25 (floor)
pub fn reward_per_block(height: u64) -> u128 {
    if height < MINING_UPGRADE_HEIGHT {
        // Legacy schedule
        let year = height / BLOCKS_PER_YEAR;
        match year {
            0 => 10_000_000_000,
            1 => 8_000_000_000,
            2 => 6_000_000_000,
            3 => 4_000_000_000,
            4..=9 => 2_000_000_000,
            _ => 1_000_000_000,
        }
    } else {
        // New geometric halving schedule from upgrade point
        let adjusted = height - MINING_UPGRADE_HEIGHT;
        let period = adjusted / HALVING_PERIOD;
        match period {
            0 => 8_000_000_000,   // 8 CLAW
            1 => 4_000_000_000,   // 4 CLAW
            2 => 2_000_000_000,   // 2 CLAW
            3 => 1_000_000_000,   // 1 CLAW
            4 => 500_000_000,     // 0.5 CLAW
            _ => 250_000_000,     // 0.25 CLAW (floor)
        }
    }
}

/// Distribute the block reward to active validators proportional to their weight.
///
/// The reward is deducted from the Node Incentive Pool. If the pool balance is
/// insufficient, the reward is capped at the remaining balance. When the pool
/// is empty, no rewards are distributed.
///
/// `validators` contains `(address, weight)` pairs for all active validators.
/// Each validator receives `reward * weight / total_weight`.
///
/// Returns a list of `BlockEvent::RewardDistributed` events for each recipient.
pub fn distribute_block_reward(
    world: &mut WorldState,
    validators: &[([u8; 32], u64)],
    height: u64,
) -> Vec<BlockEvent> {
    let mut events = Vec::new();

    if validators.is_empty() {
        return events;
    }

    let pool_addr = genesis_address(NODE_INCENTIVE_POOL_INDEX);
    let pool_balance = world.balances.get(&pool_addr).copied().unwrap_or(0);
    if pool_balance == 0 {
        return events;
    }

    let base_reward = reward_per_block(height);
    // After mining upgrade, validators only get 65% of the base reward
    let validator_reward = if height >= MINING_UPGRADE_HEIGHT {
        base_reward * VALIDATOR_REWARD_BPS / 10000
    } else {
        base_reward
    };
    // Cap at remaining pool balance
    let reward = validator_reward.min(pool_balance);
    if reward == 0 {
        return events;
    }

    let total_weight: u64 = validators.iter().fold(0u64, |acc, (_, w)| acc.saturating_add(*w));
    if total_weight == 0 {
        return events;
    }

    // Deduct full reward from pool first
    *world.balances.entry(pool_addr).or_insert(0) -= reward;

    // Distribute proportionally; track actual distributed to handle rounding
    let mut distributed: u128 = 0;
    let validator_count = validators.len();

    for (i, (addr, weight)) in validators.iter().enumerate() {
        let share = if i == validator_count - 1 {
            // Last validator gets the remainder to avoid dust loss
            reward - distributed
        } else {
            reward * (*weight as u128) / (total_weight as u128)
        };
        if share > 0 {
            let reward_recipient = world.stake_delegations.get(addr).copied().unwrap_or(*addr);
            if reward_recipient != *addr {
                // Delegated: split by commission
                let commission_bps = world.stake_commissions.get(addr).copied().unwrap_or(8000) as u128;
                let validator_share = share * commission_bps / 10000;
                let delegator_share = share - validator_share;

                if validator_share > 0 {
                    *world.balances.entry(*addr).or_insert(0) += validator_share;
                    events.push(BlockEvent::RewardDistributed {
                        recipient: *addr,
                        amount: validator_share,
                        reward_type: "validator_commission".into(),
                    });
                }
                if delegator_share > 0 {
                    *world.balances.entry(reward_recipient).or_insert(0) += delegator_share;
                    events.push(BlockEvent::RewardDistributed {
                        recipient: reward_recipient,
                        amount: delegator_share,
                        reward_type: "delegator_reward".into(),
                    });
                }
            } else {
                // Self-stake: all to validator
                *world.balances.entry(*addr).or_insert(0) += share;
                events.push(BlockEvent::RewardDistributed {
                    recipient: *addr,
                    amount: share,
                    reward_type: "block_reward".into(),
                });
            }
            distributed += share;
        }
    }

    events
}

/// Distribute transaction fees collected in a block.
///
/// Split:
/// - 50% to the block proposer
/// - 30% burned (not credited to anyone)
/// - 20% to the ecosystem fund (genesis address index 2)
///
/// Returns a list of `BlockEvent::RewardDistributed` events for each distribution.
pub fn distribute_fees(
    world: &mut WorldState,
    proposer: &[u8; 32],
    total_fees: u128,
) -> Vec<BlockEvent> {
    let mut events = Vec::new();

    if total_fees == 0 {
        return events;
    }

    let proposer_share = total_fees * 50 / 100;
    let ecosystem_share = total_fees * 20 / 100;
    let burned = total_fees - proposer_share - ecosystem_share;

    if proposer_share > 0 {
        let reward_recipient = world.stake_delegations.get(proposer).copied().unwrap_or(*proposer);
        if reward_recipient != *proposer {
            // Delegated: split by commission
            let commission_bps = world.stake_commissions.get(proposer).copied().unwrap_or(8000) as u128;
            let validator_fee = proposer_share * commission_bps / 10000;
            let delegator_fee = proposer_share - validator_fee;

            if validator_fee > 0 {
                *world.balances.entry(*proposer).or_insert(0) += validator_fee;
                events.push(BlockEvent::RewardDistributed {
                    recipient: *proposer,
                    amount: validator_fee,
                    reward_type: "proposer_fee_commission".into(),
                });
            }
            if delegator_fee > 0 {
                *world.balances.entry(reward_recipient).or_insert(0) += delegator_fee;
                events.push(BlockEvent::RewardDistributed {
                    recipient: reward_recipient,
                    amount: delegator_fee,
                    reward_type: "proposer_fee_delegator".into(),
                });
            }
        } else {
            // Self-stake: all to proposer
            *world.balances.entry(*proposer).or_insert(0) += proposer_share;
            events.push(BlockEvent::RewardDistributed {
                recipient: *proposer,
                amount: proposer_share,
                reward_type: "proposer_fee".into(),
            });
        }
    }

    if ecosystem_share > 0 {
        let eco_addr = genesis_address(ECOSYSTEM_FUND_INDEX);
        *world.balances.entry(eco_addr).or_insert(0) += ecosystem_share;
        events.push(BlockEvent::RewardDistributed {
            recipient: eco_addr,
            amount: ecosystem_share,
            reward_type: "ecosystem_fee".into(),
        });
    }

    if burned > 0 {
        events.push(BlockEvent::RewardDistributed {
            recipient: [0u8; 32],
            amount: burned,
            reward_type: "fee_burn".into(),
        });
    }

    events
}

/// Distribute the mining portion (35%) of block rewards to active miners.
///
/// Only active after MINING_UPGRADE_HEIGHT. Each active miner receives a share
/// proportional to `tier_weight * reputation_bps`. The reward is deducted from
/// the Node Incentive Pool.
///
/// Returns a list of `BlockEvent::RewardDistributed` events.
pub fn distribute_mining_rewards(
    world: &mut WorldState,
    height: u64,
) -> Vec<BlockEvent> {
    let mut events = Vec::new();

    if height < MINING_UPGRADE_HEIGHT {
        return events;
    }

    let pool_addr = genesis_address(NODE_INCENTIVE_POOL_INDEX);
    let pool_balance = world.balances.get(&pool_addr).copied().unwrap_or(0);
    if pool_balance == 0 {
        return events;
    }

    // Collect active miners with their weights
    let active_miners: Vec<([u8; 32], u128)> = world
        .miners
        .iter()
        .filter(|(_, m)| m.active)
        .map(|(addr, m)| {
            let tier_weight: u128 = match m.tier {
                claw_types::state::MinerTier::Online => 1,
            };
            let weight = tier_weight * (m.reputation_bps as u128);
            (*addr, weight)
        })
        .collect();

    if active_miners.is_empty() {
        return events;
    }

    let base_reward = reward_per_block(height);
    let mining_reward = base_reward * MINING_REWARD_BPS / 10000;
    let reward = mining_reward.min(pool_balance);
    if reward == 0 {
        return events;
    }

    let total_weight: u128 = active_miners.iter().map(|(_, w)| *w).sum();
    if total_weight == 0 {
        return events;
    }

    // Deduct from pool
    *world.balances.entry(pool_addr).or_insert(0) -= reward;

    // Distribute proportionally
    let mut distributed: u128 = 0;
    let miner_count = active_miners.len();

    for (i, (addr, weight)) in active_miners.iter().enumerate() {
        let share = if i == miner_count - 1 {
            reward - distributed
        } else {
            reward * weight / total_weight
        };
        if share > 0 {
            *world.balances.entry(*addr).or_insert(0) += share;
            events.push(BlockEvent::RewardDistributed {
                recipient: *addr,
                amount: share,
                reward_type: "mining_reward".into(),
            });
            distributed += share;
        }
    }

    events
}

/// Mark miners as inactive if they haven't sent a heartbeat within the grace period.
/// Used by V1 heartbeat logic (before HEARTBEAT_V2_HEIGHT).
pub fn update_miner_activity(world: &mut WorldState, current_height: u64) {
    if current_height >= HEARTBEAT_V2_HEIGHT {
        return; // V2 handles deactivation via epoch boundary processing
    }
    for miner in world.miners.values_mut() {
        if miner.active && miner.last_heartbeat + MINER_GRACE_BLOCKS < current_height {
            miner.active = false;
        }
    }
}

// --- Heartbeat V2: Epoch-based mining rewards ---

/// Accumulate the mining share of block rewards into the epoch bucket.
///
/// Called every block (replaces direct `distribute_mining_rewards` after V2 activation).
/// The bucket is distributed at epoch boundaries by `process_miner_epoch_boundary`.
pub fn accumulate_mining_reward(world: &mut WorldState, height: u64) -> Vec<BlockEvent> {
    if height < HEARTBEAT_V2_HEIGHT {
        return distribute_mining_rewards(world, height);
    }

    if height < MINING_UPGRADE_HEIGHT {
        return Vec::new();
    }

    let pool_addr = genesis_address(NODE_INCENTIVE_POOL_INDEX);
    let pool_balance = world.balances.get(&pool_addr).copied().unwrap_or(0);
    if pool_balance == 0 {
        return Vec::new();
    }

    let base_reward = reward_per_block(height);
    let mining_reward = base_reward * MINING_REWARD_BPS / 10000;
    let actual = mining_reward.min(pool_balance);
    if actual == 0 {
        return Vec::new();
    }

    // Deduct from pool, accumulate into epoch bucket
    *world.balances.entry(pool_addr).or_insert(0) -= actual;
    world.epoch_reward_bucket += actual;

    Vec::new() // No per-block events; rewards are emitted at epoch boundary
}

/// Process epoch boundary: settle pending rewards, update attendance, decay reputation.
///
/// Must be called at every block where `height % MINER_EPOCH_LENGTH == 0` and
/// `height >= HEARTBEAT_V2_HEIGHT`. Must run BEFORE `state_root()` computation.
///
/// At the activation boundary (height == HEARTBEAT_V2_HEIGHT), performs normalization
/// only — skips settlement to avoid penalizing V1-compliant miners.
pub fn process_miner_epoch_boundary(world: &mut WorldState, height: u64) -> Vec<BlockEvent> {
    let mut events = Vec::new();

    if height < HEARTBEAT_V2_HEIGHT {
        return events;
    }
    if height % MINER_EPOCH_LENGTH != 0 {
        return events;
    }

    let current_epoch = height / MINER_EPOCH_LENGTH;
    let pool_addr = genesis_address(NODE_INCENTIVE_POOL_INDEX);
    let is_activation = height == HEARTBEAT_V2_HEIGHT;

    // === ACTIVATION BOUNDARY: normalize only, skip settlement ===
    if is_activation {
        // 1. Normalize all miners' V2 fields to deterministic zeros.
        //    This ensures all nodes agree regardless of upgrade timing.
        for miner in world.miners.values_mut() {
            miner.pending_rewards = 0;
            miner.pending_epoch = 0;
            miner.epoch_attendance = 0;
            miner.consecutive_misses = 0;
        }

        // 2. Return any bucket accumulation from the activation block to pool.
        //    (accumulate_mining_reward runs before this function in chain.rs)
        if world.epoch_reward_bucket > 0 {
            *world.balances.entry(pool_addr).or_insert(0) += world.epoch_reward_bucket;
            world.epoch_reward_bucket = 0;
        }

        // 3. Clear epoch checkins and return — no settlement on activation.
        //    Reason: V1 miners have last_heartbeat at 1000-block intervals,
        //    which doesn't match 100-block epoch windows. Settling now would
        //    incorrectly penalize all compliant V1 miners.
        world.epoch_checkins.clear();
        return events;
    }

    // === NORMAL SETTLEMENT (from second V2 epoch onward) ===

    // The settled epoch is the one before the current boundary.
    let settled_epoch = current_epoch - 1;

    // --- Phase 1: Settle previous epoch's pending + update attendance ---
    let miner_addrs: Vec<[u8; 32]> = world.miners.keys().copied().collect();
    for addr in &miner_addrs {
        // Determine if the miner checked in during the settled epoch.
        // V3: use last_checkin_epoch (set by P2P witness inclusion).
        // Pre-V3: use last_heartbeat / MINER_EPOCH_LENGTH (set by heartbeat tx).
        let miner = world.miners.get(addr).unwrap();
        let checked_in = if height >= CHECKIN_V3_HEIGHT {
            miner.last_checkin_epoch == settled_epoch
        } else {
            miner.last_heartbeat / MINER_EPOCH_LENGTH == settled_epoch
        };

        let miner = world.miners.get_mut(addr).unwrap();

        if checked_in {
            // Confirm previous epoch's pending rewards
            if miner.pending_epoch == settled_epoch && miner.pending_rewards > 0 {
                let confirmed = miner.pending_rewards;
                *world.balances.entry(*addr).or_insert(0) += confirmed;
                events.push(BlockEvent::RewardDistributed {
                    recipient: *addr,
                    amount: confirmed,
                    reward_type: "mining_reward_confirmed".into(),
                });
            }

            // Reactivate + reset counters
            miner.active = true;
            miner.consecutive_misses = 0;
            miner.epoch_attendance = (miner.epoch_attendance << 1) | 1;

            // Upgrade reputation based on miner age
            let age = height.saturating_sub(miner.registered_at);
            if age >= BLOCKS_30_DAYS {
                miner.reputation_bps = miner.reputation_bps.max(REPUTATION_VETERAN_BPS);
            } else if age >= BLOCKS_7_DAYS {
                miner.reputation_bps = miner.reputation_bps.max(REPUTATION_ESTABLISHED_BPS);
            }
        } else {
            // Forfeit previous epoch's pending rewards → return to pool
            if miner.pending_rewards > 0 {
                let forfeited = miner.pending_rewards;
                *world.balances.entry(pool_addr).or_insert(0) += forfeited;
                events.push(BlockEvent::RewardDistributed {
                    recipient: pool_addr,
                    amount: forfeited,
                    reward_type: "mining_reward_forfeited".into(),
                });
            }

            // Absence penalty
            miner.consecutive_misses = miner.consecutive_misses.saturating_add(1);
            miner.epoch_attendance = miner.epoch_attendance << 1; // LSB = 0

            // Reputation decay: -1% per missed epoch, floor at NEWCOMER
            let decayed = (miner.reputation_bps as u32) * (10000 - REPUTATION_DECAY_BPS as u32) / 10000;
            miner.reputation_bps = (decayed as u16).max(REPUTATION_NEWCOMER_BPS);

            // Deactivate after too many consecutive misses
            if miner.consecutive_misses >= MINER_GRACE_EPOCHS {
                miner.active = false;
            }
        }

        // Reset pending for this epoch (will be filled in Phase 2)
        let miner = world.miners.get_mut(addr).unwrap();
        miner.pending_rewards = 0;
        miner.pending_epoch = current_epoch;
    }

    // --- Phase 2: Distribute epoch bucket to qualified miners as pending ---
    let bucket = world.epoch_reward_bucket;
    if bucket > 0 {
        // Collect qualified miners with their weights
        let qualified: Vec<([u8; 32], u128)> = world
            .miners
            .iter()
            .filter(|(_, m)| m.active)
            .filter_map(|(addr, m)| {
                let attendance_bits = m.epoch_attendance & 0x0FFF; // low 12 bits
                let count = attendance_bits.count_ones();
                if count < MINER_MIN_UPTIME_FOR_REWARD {
                    return None;
                }
                let uptime_mult = miner_uptime_multiplier(count);
                let tier_weight: u128 = match m.tier {
                    MinerTier::Online => 1,
                };
                let weight = tier_weight * (m.reputation_bps as u128) * uptime_mult;
                if weight > 0 {
                    Some((*addr, weight))
                } else {
                    None
                }
            })
            .collect();

        if qualified.is_empty() {
            // No qualified miners — return bucket to pool
            *world.balances.entry(pool_addr).or_insert(0) += bucket;
        } else {
            let total_weight: u128 = qualified.iter().map(|(_, w)| *w).sum();
            let mut distributed: u128 = 0;
            let count = qualified.len();

            for (i, (addr, weight)) in qualified.iter().enumerate() {
                let share = if i == count - 1 {
                    bucket - distributed
                } else {
                    bucket * weight / total_weight
                };
                if share > 0 {
                    world.miners.get_mut(addr).unwrap().pending_rewards = share;
                    distributed += share;
                }
            }
        }

        world.epoch_reward_bucket = 0;
    }

    // --- Phase 3: Clear epoch check-ins ---
    world.epoch_checkins.clear();

    events
}

/// One-time V2→V3 migration: convert last_heartbeat to last_checkin_epoch.
/// Called exactly once at CHECKIN_V3_HEIGHT, BEFORE the first V3 settlement.
/// Ensures miners who checked in during the last V2 epoch are credited.
pub fn migrate_miners_v2_to_v3(world: &mut WorldState) {
    for miner in world.miners.values_mut() {
        miner.last_checkin_epoch = miner.last_heartbeat / MINER_EPOCH_LENGTH;
    }
    tracing::info!(
        miners = world.miners.len(),
        "V2→V3 migration: last_heartbeat → last_checkin_epoch"
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_reward_schedule_legacy() {
        // Legacy schedule applies before MINING_UPGRADE_HEIGHT
        assert_eq!(reward_per_block(0), 10_000_000_000);
        assert_eq!(reward_per_block(MINING_UPGRADE_HEIGHT - 1), 10_000_000_000);
    }

    #[test]
    fn test_reward_schedule_new() {
        // New geometric halving schedule after MINING_UPGRADE_HEIGHT
        assert_eq!(reward_per_block(MINING_UPGRADE_HEIGHT), 8_000_000_000);
        assert_eq!(reward_per_block(MINING_UPGRADE_HEIGHT + HALVING_PERIOD - 1), 8_000_000_000);
        assert_eq!(reward_per_block(MINING_UPGRADE_HEIGHT + HALVING_PERIOD), 4_000_000_000);
        assert_eq!(reward_per_block(MINING_UPGRADE_HEIGHT + 2 * HALVING_PERIOD), 2_000_000_000);
        assert_eq!(reward_per_block(MINING_UPGRADE_HEIGHT + 3 * HALVING_PERIOD), 1_000_000_000);
        assert_eq!(reward_per_block(MINING_UPGRADE_HEIGHT + 4 * HALVING_PERIOD), 500_000_000);
        assert_eq!(reward_per_block(MINING_UPGRADE_HEIGHT + 5 * HALVING_PERIOD), 250_000_000);
        assert_eq!(reward_per_block(MINING_UPGRADE_HEIGHT + 6 * HALVING_PERIOD), 250_000_000);
    }

    #[test]
    fn test_distribute_block_reward_basic() {
        let mut world = WorldState::default();
        let pool_addr = genesis_address(NODE_INCENTIVE_POOL_INDEX);
        world.balances.insert(pool_addr, 100_000_000_000); // 100 CLAW

        let v1 = [1u8; 32];
        let v2 = [2u8; 32];
        let validators = vec![(v1, 70), (v2, 30)];

        distribute_block_reward(&mut world, &validators, 0);

        // Reward at height 0 = 10 CLAW
        let pool_after = world.balances.get(&pool_addr).copied().unwrap_or(0);
        assert_eq!(pool_after, 90_000_000_000); // 100 - 10 = 90

        let v1_bal = world.balances.get(&v1).copied().unwrap_or(0);
        let v2_bal = world.balances.get(&v2).copied().unwrap_or(0);
        assert_eq!(v1_bal + v2_bal, 10_000_000_000);
        assert_eq!(v1_bal, 7_000_000_000); // 70%
        assert_eq!(v2_bal, 3_000_000_000); // 30%
    }

    #[test]
    fn test_distribute_block_reward_caps_at_pool() {
        let mut world = WorldState::default();
        let pool_addr = genesis_address(NODE_INCENTIVE_POOL_INDEX);
        world.balances.insert(pool_addr, 3_000_000_000); // 3 CLAW (less than 10 CLAW reward)

        let v1 = [1u8; 32];
        let validators = vec![(v1, 100)];

        distribute_block_reward(&mut world, &validators, 0);

        assert_eq!(world.balances.get(&pool_addr).copied().unwrap_or(0), 0);
        assert_eq!(world.balances.get(&v1).copied().unwrap_or(0), 3_000_000_000);
    }

    #[test]
    fn test_distribute_block_reward_empty_pool() {
        let mut world = WorldState::default();
        let v1 = [1u8; 32];
        let validators = vec![(v1, 100)];

        distribute_block_reward(&mut world, &validators, 0);

        assert_eq!(world.balances.get(&v1).copied().unwrap_or(0), 0);
    }

    #[test]
    fn test_distribute_fees() {
        let mut world = WorldState::default();
        let proposer = [1u8; 32];
        let eco_addr = genesis_address(ECOSYSTEM_FUND_INDEX);

        distribute_fees(&mut world, &proposer, 10_000_000);

        // 50% to proposer
        assert_eq!(world.balances.get(&proposer).copied().unwrap_or(0), 5_000_000);
        // 20% to ecosystem
        assert_eq!(world.balances.get(&eco_addr).copied().unwrap_or(0), 2_000_000);
        // 30% burned (total accounted: 5M + 2M = 7M, burned = 3M)
    }

    #[test]
    fn test_distribute_fees_zero() {
        let mut world = WorldState::default();
        let proposer = [1u8; 32];

        distribute_fees(&mut world, &proposer, 0);

        assert_eq!(world.balances.get(&proposer).copied().unwrap_or(0), 0);
    }
}
