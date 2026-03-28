//! Agent Score computation — multi-dimensional, on-chain behavior based.
//!
//! The Agent Score replaces the old subjective ReputationAttest-based scoring.
//! It is computed from five dimensions:
//!
//! - **Activity** (30%): on-chain transaction activity
//! - **Uptime** (25%): validator block-signing rate
//! - **Block Production** (20%): validator block-production rate
//! - **Economic** (15%): staking, holding, gas contribution
//! - **Platform** (10%): PlatformActivityReport data
//!
//! Non-validators have uptime and block_production = 0; their three remaining
//! dimensions are re-normalized to (activity 55%, economic 27%, platform 18%).
//!
//! Time decay: `decay = 0.5 ^ (age_epochs / 2880)` where age_epochs is how
//! many epochs have passed since the agent registered.
//!
//! Final score is clamped to [0, 10000].

use crate::world::WorldState;

/// Detailed agent score breakdown returned by the RPC.
#[derive(Debug, Clone, Default)]
pub struct AgentScoreDetail {
    /// Total composite score [0, 10000].
    pub total: u64,
    /// Activity sub-score [0, 10000].
    pub activity: u64,
    /// Uptime sub-score [0, 10000] (validators only).
    pub uptime: u64,
    /// Block production sub-score [0, 10000] (validators only).
    pub block_production: u64,
    /// Economic sub-score [0, 10000].
    pub economic: u64,
    /// Platform sub-score [0, 10000].
    pub platform: u64,
    /// Time decay factor as basis points [0, 10000] where 10000 = 1.0.
    pub decay_factor: u64,
}

/// Compute the multi-dimensional Agent Score for a given address.
pub fn compute_agent_score(state: &WorldState, address: &[u8; 32]) -> AgentScoreDetail {
    let activity = compute_activity_score(state, address);
    let uptime = compute_uptime_score(state, address);
    let block_prod = compute_block_production_score(state, address);
    let economic = compute_economic_score(state, address);
    let platform = compute_platform_score(state, address);
    let decay = compute_decay_factor(state, address);

    // Clamp each sub-score to [0, 10000] before computing the weighted average.
    // Some sub-scores (especially economic) can exceed 10000 when the address is
    // not in the agents registry, causing the normalization denominator to be too
    // small. Without clamping, a single inflated dimension would dominate the total.
    let activity = activity.min(10000);
    let uptime = uptime.min(10000);
    let block_prod = block_prod.min(10000);
    let economic = economic.min(10000);
    let platform = platform.min(10000);

    let is_validator = state.stakes.get(address).copied().unwrap_or(0) >= 10_000_000_000_000
        && (uptime > 0 || block_prod > 0);

    let raw_total = if is_validator {
        // Validator: 5-dimensional weighted average
        (activity * 30 + uptime * 25 + block_prod * 20 + economic * 15 + platform * 10) / 100
    } else {
        // Non-validator: 3-dimensional re-normalized (55/27/18)
        (activity * 55 + economic * 27 + platform * 18) / 100
    };

    // Apply decay
    let decayed = (raw_total * decay) / 10000;
    let total = decayed.min(10000);

    AgentScoreDetail {
        total,
        activity,
        uptime,
        block_production: block_prod,
        economic,
        platform,
        decay_factor: decay,
    }
}

/// Activity score: based on tx_count, contract interactions, etc.
/// Normalized to [0, 10000].
fn compute_activity_score(state: &WorldState, address: &[u8; 32]) -> u64 {
    let stats = match state.activity_stats.get(address) {
        Some(s) => s,
        None => return 0,
    };

    // Weighted activity: tx_count*1 + contract_deploys*10 + contract_calls*3 +
    // tokens_created*5 + services_registered*5
    let weighted = stats.tx_count as u64
        + stats.contract_deploys as u64 * 10
        + stats.contract_calls as u64 * 3
        + stats.tokens_created as u64 * 5
        + stats.services_registered as u64 * 5;

    // Find the max across all addresses for normalization
    let max_weighted = state.activity_stats.values().map(|s| {
        s.tx_count as u64
            + s.contract_deploys as u64 * 10
            + s.contract_calls as u64 * 3
            + s.tokens_created as u64 * 5
            + s.services_registered as u64 * 5
    }).max().unwrap_or(1).max(1);

    (weighted * 10000) / max_weighted
}

/// Uptime score: based on signed_blocks / expected_blocks ratio.
/// Returns [0, 10000].
fn compute_uptime_score(state: &WorldState, address: &[u8; 32]) -> u64 {
    let uptime = match state.validator_uptime.get(address) {
        Some(u) => u,
        None => return 0,
    };

    if uptime.expected_blocks == 0 {
        return 0;
    }

    ((uptime.signed_blocks * 10000) / uptime.expected_blocks).min(10000)
}

/// Block production score: based on produced_blocks / expected_blocks ratio.
/// Returns [0, 10000].
fn compute_block_production_score(state: &WorldState, address: &[u8; 32]) -> u64 {
    let uptime = match state.validator_uptime.get(address) {
        Some(u) => u,
        None => return 0,
    };

    if uptime.expected_blocks == 0 {
        return 0;
    }

    ((uptime.produced_blocks * 10000) / uptime.expected_blocks).min(10000)
}

/// Economic score: based on stake amount + CLAW balance + gas contribution.
/// Normalized to [0, 10000].
fn compute_economic_score(state: &WorldState, address: &[u8; 32]) -> u64 {
    let stake = state.stakes.get(address).copied().unwrap_or(0);
    let balance = state.balances.get(address).copied().unwrap_or(0);
    let gas = state.activity_stats.get(address)
        .map(|s| s.gas_consumed)
        .unwrap_or(0);

    // Weighted economic value: stake*3 + balance*1 + gas*2
    let value = (stake / 1_000_000_000) as u64 * 3
        + (balance / 1_000_000_000) as u64
        + gas / 1_000_000 * 2;

    // Find max across ALL addresses with stake or balance (not just agents),
    // so that validators who are not registered agents still get a correctly
    // normalized score.
    let all_addresses = state.stakes.keys().chain(state.balances.keys()).collect::<std::collections::BTreeSet<_>>();
    let max_value = all_addresses.iter().map(|addr| {
        let s = state.stakes.get(*addr).copied().unwrap_or(0);
        let b = state.balances.get(*addr).copied().unwrap_or(0);
        let g = state.activity_stats.get(*addr)
            .map(|st| st.gas_consumed)
            .unwrap_or(0);
        (s / 1_000_000_000) as u64 * 3
            + (b / 1_000_000_000) as u64
            + g / 1_000_000 * 2
    }).max().unwrap_or(1).max(1);

    (value * 10000) / max_value
}

/// Platform score: based on aggregated PlatformActivityReport data.
/// Normalized to [0, 10000].
fn compute_platform_score(state: &WorldState, address: &[u8; 32]) -> u64 {
    let agg = match state.platform_activity.get(address) {
        Some(a) => a,
        None => return 0,
    };

    // Weighted: total_actions + platform_count * 100
    let value = agg.total_actions + agg.platform_count as u64 * 100;

    let max_value = state.platform_activity.values().map(|a| {
        a.total_actions + a.platform_count as u64 * 100
    }).max().unwrap_or(1).max(1);

    (value * 10000) / max_value
}

/// Time decay factor: 0.5 ^ (age_epochs / 2880).
/// Returns basis points [0, 10000] where 10000 = 1.0 (no decay).
///
/// Uses integer approximation: for each full half-life (2880 epochs),
/// the factor halves. For partial periods, linear interpolation is used.
fn compute_decay_factor(state: &WorldState, address: &[u8; 32]) -> u64 {
    let agent = match state.agents.get(address) {
        Some(a) => a,
        None => return 10000, // No decay for unregistered (shouldn't happen)
    };

    let current_epoch = state.block_height / 100;
    let registered_epoch = agent.registered_at / 100;
    let age_epochs = current_epoch.saturating_sub(registered_epoch);

    if age_epochs == 0 {
        return 10000;
    }

    // Half-life = 2880 epochs (~10 days at 100 blocks/epoch, 3s block time)
    let half_lives = age_epochs / 2880;
    let remainder = age_epochs % 2880;

    // Base decay from full half-lives (0.5^n)
    // Cap at 10 half-lives (factor ~ 0.001)
    let base = if half_lives >= 10 {
        10 // 10000 / 1024 ≈ 10
    } else {
        10000u64 >> half_lives
    };

    // Linear interpolation for the remainder within the current half-life
    // factor = base * (1 - remainder / 2880 * 0.5) = base * (2880 - remainder/2) / 2880
    let interpolated = base * (2880 * 2 - remainder) / (2880 * 2);

    interpolated.max(1) // Minimum factor of 1 bps (0.01%)
}

#[cfg(test)]
mod tests {
    use super::*;
    use claw_types::state::{ActivityStats, AgentIdentity, ValidatorUptime, PlatformActivityAgg};
    use std::collections::BTreeMap;

    fn make_address(seed: u8) -> [u8; 32] {
        let mut addr = [0u8; 32];
        addr[0] = seed;
        addr
    }

    fn setup_state_with_agent(seed: u8) -> (WorldState, [u8; 32]) {
        let addr = make_address(seed);
        let mut state = WorldState::default();
        state.block_height = 1000;
        state.agents.insert(addr, AgentIdentity {
            address: addr,
            name: format!("agent-{seed}"),
            metadata: BTreeMap::new(),
            registered_at: 0,
        });
        state.balances.insert(addr, 100_000_000_000_000); // 100k CLAW
        (state, addr)
    }

    #[test]
    fn test_activity_score_zero_with_no_stats() {
        let (state, addr) = setup_state_with_agent(1);
        let score = compute_agent_score(&state, &addr);
        assert_eq!(score.activity, 0);
    }

    #[test]
    fn test_activity_score_with_stats() {
        let (mut state, addr) = setup_state_with_agent(1);
        state.activity_stats.insert(addr, ActivityStats {
            tx_count: 50,
            contract_deploys: 2,
            contract_calls: 10,
            tokens_created: 1,
            services_registered: 1,
            gas_consumed: 50_000_000,
        });
        let score = compute_agent_score(&state, &addr);
        assert_eq!(score.activity, 10000); // Only agent, so normalized to max
    }

    #[test]
    fn test_non_validator_three_dimensions() {
        let (mut state, addr) = setup_state_with_agent(1);
        state.activity_stats.insert(addr, ActivityStats {
            tx_count: 10,
            contract_deploys: 0,
            contract_calls: 5,
            tokens_created: 0,
            services_registered: 0,
            gas_consumed: 10_000_000,
        });
        let score = compute_agent_score(&state, &addr);
        // Non-validator: uptime and block_production should be 0
        assert_eq!(score.uptime, 0);
        assert_eq!(score.block_production, 0);
        // Total should be > 0
        assert!(score.total > 0);
    }

    #[test]
    fn test_validator_five_dimensions() {
        let (mut state, addr) = setup_state_with_agent(1);
        state.stakes.insert(addr, 50_000_000_000_000); // 50k CLAW staked
        state.activity_stats.insert(addr, ActivityStats {
            tx_count: 100,
            contract_deploys: 5,
            contract_calls: 50,
            tokens_created: 2,
            services_registered: 3,
            gas_consumed: 100_000_000,
        });
        state.validator_uptime.insert(addr, ValidatorUptime {
            signed_blocks: 9500,
            expected_blocks: 10000,
            produced_blocks: 900,
        });
        state.platform_activity.insert(addr, PlatformActivityAgg {
            total_actions: 1000,
            platform_count: 5,
        });

        let score = compute_agent_score(&state, &addr);
        assert!(score.total > 0);
        assert_eq!(score.uptime, 9500); // 9500/10000 * 10000
        assert_eq!(score.block_production, 900); // 900/10000 * 10000
        // registered_at=0, block_height=1000 -> 10 epochs, slight decay
        assert!(score.decay_factor > 9900, "decay_factor={}", score.decay_factor);
        assert!(score.total > 0);
    }

    #[test]
    fn test_decay_factor_no_decay() {
        let (state, addr) = setup_state_with_agent(1);
        let decay = compute_decay_factor(&state, &addr);
        // registered_at=0, block_height=1000 -> 10 epochs, well under 2880
        assert!(decay > 9900);
    }

    #[test]
    fn test_decay_factor_one_half_life() {
        let (mut state, addr) = setup_state_with_agent(1);
        // Set block height to 2880 * 100 = 288000 blocks (1 half-life)
        state.block_height = 288_000;
        let decay = compute_decay_factor(&state, &addr);
        // Should be approximately 5000 (half)
        assert!(decay >= 4900 && decay <= 5100, "decay={decay}");
    }

    #[test]
    fn test_score_clamped_to_10000() {
        let (mut state, addr) = setup_state_with_agent(1);
        state.stakes.insert(addr, 1_000_000_000_000_000); // Huge stake
        state.activity_stats.insert(addr, ActivityStats {
            tx_count: 10000,
            contract_deploys: 100,
            contract_calls: 1000,
            tokens_created: 50,
            services_registered: 50,
            gas_consumed: 10_000_000_000,
        });
        state.validator_uptime.insert(addr, ValidatorUptime {
            signed_blocks: 10000,
            expected_blocks: 10000,
            produced_blocks: 10000,
        });
        state.platform_activity.insert(addr, PlatformActivityAgg {
            total_actions: 100000,
            platform_count: 100,
        });

        let score = compute_agent_score(&state, &addr);
        assert!(score.total <= 10000);
    }
}
