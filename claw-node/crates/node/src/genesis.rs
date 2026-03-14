//! Genesis block generation.

use claw_types::block::Block;
use claw_state::WorldState;
use claw_types::state::CLW_TOTAL_SUPPLY;

/// Genesis allocation addresses (deterministic from index).
fn genesis_address(index: u8) -> [u8; 32] {
    let mut addr = [0u8; 32];
    addr[0] = index;
    addr
}

/// Create the genesis state with initial token distribution.
pub fn create_genesis_state() -> WorldState {
    let mut state = WorldState::default();
    state.block_height = 0;

    // Token distribution (percentages of CLW_TOTAL_SUPPLY):
    // 40% Node Incentives, 25% Ecosystem, 15% Team, 10% Early Contributors, 10% Liquidity
    let allocations: [(u8, u128); 5] = [
        (1, CLW_TOTAL_SUPPLY * 40 / 100), // Node Incentives Pool
        (2, CLW_TOTAL_SUPPLY * 25 / 100), // Ecosystem Fund
        (3, CLW_TOTAL_SUPPLY * 15 / 100), // Team (locked)
        (4, CLW_TOTAL_SUPPLY * 10 / 100), // Early Contributors
        (5, CLW_TOTAL_SUPPLY * 10 / 100), // Liquidity Reserve
    ];

    for (index, amount) in allocations {
        state.balances.insert(genesis_address(index), amount);
    }

    state
}

/// Create the genesis block.
pub fn create_genesis_block(state: &WorldState) -> Block {
    let state_root = state.state_root();
    let mut block = Block {
        height: 0,
        prev_hash: [0u8; 32],
        timestamp: 1741737600, // 2025-03-12 00:00:00 UTC (symbolic)
        validator: [0u8; 32],
        transactions: vec![],
        state_root,
        hash: [0u8; 32],
        signatures: Vec::new(),
    };
    block.hash = block.compute_hash();
    block
}
