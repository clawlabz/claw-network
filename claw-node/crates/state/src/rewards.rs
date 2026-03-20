//! Block reward distribution for ClawNetwork.
//!
//! Rewards are funded from the Node Incentive Pool (genesis address index 1)
//! and follow a decay schedule over 10+ years.

use claw_types::block::BlockEvent;
use crate::WorldState;

/// Blocks per year assuming 3-second block time: 365.25 * 86400 / 3.
pub const BLOCKS_PER_YEAR: u64 = 10_512_000;

/// Genesis address index for the Node Incentive Pool (40% of total supply).
pub const NODE_INCENTIVE_POOL_INDEX: u8 = 1;

/// Genesis address index for the Ecosystem Fund (25% of total supply).
pub const ECOSYSTEM_FUND_INDEX: u8 = 2;

/// Derive a genesis address from its allocation index.
/// Must match the derivation in `crate::genesis`.
fn genesis_address(index: u8) -> [u8; 32] {
    let mut addr = [0u8; 32];
    addr[0] = index;
    addr
}

/// Calculate the block reward (in base units, 9 decimals) for a given height.
///
/// Decay schedule:
/// - Year 1  (blocks 0 ..  10_511_999): 10 CLAW = 10_000_000_000
/// - Year 2  (blocks 10_512_000 ..  21_023_999):  8 CLAW =  8_000_000_000
/// - Year 3  (blocks 21_024_000 ..  31_535_999):  6 CLAW =  6_000_000_000
/// - Year 4  (blocks 31_536_000 ..  42_047_999):  4 CLAW =  4_000_000_000
/// - Year 5-10 (blocks 42_048_000 .. 105_119_999): 2 CLAW =  2_000_000_000
/// - Year 11+  (blocks 105_120_000 ..):            1 CLAW =  1_000_000_000
pub fn reward_per_block(height: u64) -> u128 {
    let year = height / BLOCKS_PER_YEAR;
    match year {
        0 => 10_000_000_000,
        1 => 8_000_000_000,
        2 => 6_000_000_000,
        3 => 4_000_000_000,
        4..=9 => 2_000_000_000,
        _ => 1_000_000_000,
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

    let raw_reward = reward_per_block(height);
    // Cap at remaining pool balance
    let reward = raw_reward.min(pool_balance);
    if reward == 0 {
        return events;
    }

    let total_weight: u64 = validators.iter().map(|(_, w)| *w).sum();
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
                let commission_bps = world.stake_commissions.get(addr).copied().unwrap_or(10000) as u128;
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
            let commission_bps = world.stake_commissions.get(proposer).copied().unwrap_or(10000) as u128;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_reward_schedule() {
        // Year 1
        assert_eq!(reward_per_block(0), 10_000_000_000);
        assert_eq!(reward_per_block(BLOCKS_PER_YEAR - 1), 10_000_000_000);

        // Year 2
        assert_eq!(reward_per_block(BLOCKS_PER_YEAR), 8_000_000_000);
        assert_eq!(reward_per_block(2 * BLOCKS_PER_YEAR - 1), 8_000_000_000);

        // Year 3
        assert_eq!(reward_per_block(2 * BLOCKS_PER_YEAR), 6_000_000_000);

        // Year 4
        assert_eq!(reward_per_block(3 * BLOCKS_PER_YEAR), 4_000_000_000);

        // Year 5-10
        assert_eq!(reward_per_block(4 * BLOCKS_PER_YEAR), 2_000_000_000);
        assert_eq!(reward_per_block(9 * BLOCKS_PER_YEAR), 2_000_000_000);

        // Year 11+
        assert_eq!(reward_per_block(10 * BLOCKS_PER_YEAR), 1_000_000_000);
        assert_eq!(reward_per_block(100 * BLOCKS_PER_YEAR), 1_000_000_000);
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
