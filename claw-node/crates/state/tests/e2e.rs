//! End-to-end integration test for the full transaction pipeline.
//!
//! Tests all 6 TX types through WorldState, simulating what
//! the chain engine does for each block.

use claw_crypto::keys::generate_keypair;
use claw_crypto::signer::sign_transaction;
use claw_state::WorldState;
use claw_types::state::*;
use claw_types::transaction::*;
use std::collections::BTreeMap;

fn make_tx(
    sk: &claw_crypto::ed25519_dalek::SigningKey,
    nonce: u64,
    tx_type: TxType,
    payload: &impl borsh::BorshSerialize,
) -> Transaction {
    let mut tx = Transaction {
        tx_type,
        from: [0u8; 32],
        nonce,
        payload: borsh::to_vec(payload).unwrap(),
        signature: [0u8; 64],
    };
    sign_transaction(&mut tx, sk);
    tx
}

#[test]
fn full_e2e_all_six_tx_types() {
    let (sk1, vk1) = generate_keypair();
    let addr1 = vk1.to_bytes();
    let (sk2, vk2) = generate_keypair();
    let addr2 = vk2.to_bytes();

    let mut state = WorldState::default();
    state.block_height = 1;
    state.balances.insert(addr1, 10_000_000_000); // 10 CLAW
    state.balances.insert(addr2, 10_000_000_000);

    // === TX 1: Agent Register (addr1) ===
    let tx = make_tx(&sk1, 1, TxType::AgentRegister, &AgentRegisterPayload {
        name: "agent-alpha".into(),
        metadata: {
            let mut m = BTreeMap::new();
            m.insert("role".into(), "validator".into());
            m
        },
    });
    state.apply_tx(&tx, 0).unwrap();
    assert_eq!(state.agents.get(&addr1).unwrap().name, "agent-alpha");

    // === TX 2: Agent Register (addr2) ===
    let tx = make_tx(&sk2, 1, TxType::AgentRegister, &AgentRegisterPayload {
        name: "agent-beta".into(),
        metadata: BTreeMap::new(),
    });
    state.apply_tx(&tx, 0).unwrap();
    assert_eq!(state.agents.len(), 2);

    // === TX 3: Token Transfer (CLAW: addr1 → addr2) ===
    let initial1 = state.get_balance(&addr1);
    let initial2 = state.get_balance(&addr2);
    let tx = make_tx(&sk1, 2, TxType::TokenTransfer, &TokenTransferPayload {
        to: addr2,
        amount: 2_000_000_000, // 2 CLAW
    });
    state.apply_tx(&tx, 0).unwrap();
    assert_eq!(state.get_balance(&addr2), initial2 + 2_000_000_000);
    assert_eq!(state.get_balance(&addr1), initial1 - 2_000_000_000 - GAS_FEE);

    // === TX 4: Token Create (addr1 creates custom token) ===
    let tx = make_tx(&sk1, 3, TxType::TokenCreate, &TokenCreatePayload {
        name: "AlphaCredit".into(),
        symbol: "ALC".into(),
        decimals: 6,
        total_supply: 1_000_000_000_000, // 1M tokens
    });
    state.apply_tx(&tx, 0).unwrap();
    assert_eq!(state.tokens.len(), 1);
    let token_id = *state.tokens.keys().next().unwrap();
    assert_eq!(state.get_token_balance(&addr1, &token_id), 1_000_000_000_000);

    // === TX 5: Token Mint Transfer (custom token: addr1 → addr2) ===
    let tx = make_tx(&sk1, 4, TxType::TokenMintTransfer, &TokenMintTransferPayload {
        token_id,
        to: addr2,
        amount: 500_000_000_000, // 500K tokens
    });
    state.apply_tx(&tx, 0).unwrap();
    assert_eq!(state.get_token_balance(&addr1, &token_id), 500_000_000_000);
    assert_eq!(state.get_token_balance(&addr2, &token_id), 500_000_000_000);

    // === TX 6: Reputation Attest (addr1 → addr2) — deprecated, must be rejected ===
    // nonce 5 is consumed by the attempt but fails; addr1 nonce stays at 4.
    let tx = make_tx(&sk1, 5, TxType::ReputationAttest, &ReputationAttestPayload {
        to: addr2,
        category: "collaboration".into(),
        score: 85,
        platform: "e2e-test".into(),
        memo: "reliable partner".into(),
    });
    assert!(state.apply_tx(&tx, 0).is_err(), "ReputationAttest is deprecated and must be rejected");
    assert_eq!(state.reputation.len(), 0);

    // === TX 7: Service Register (addr1, nonce 5 — previous rep attest did not advance nonce) ===
    let tx = make_tx(&sk1, 5, TxType::ServiceRegister, &ServiceRegisterPayload {
        service_type: "code-review".into(),
        description: "Automated code review powered by AI".into(),
        price_token: NATIVE_TOKEN_ID,
        price_amount: 5_000_000, // 0.005 CLAW
        endpoint: "https://agent-alpha.example.com/review".into(),
        active: true,
    });
    state.apply_tx(&tx, 0).unwrap();
    assert_eq!(state.services.len(), 1);

    // === TX 8: Reputation attempt from addr2 — also deprecated, must be rejected ===
    let tx = make_tx(&sk2, 2, TxType::ReputationAttest, &ReputationAttestPayload {
        to: addr1,
        category: "service".into(),
        score: 92,
        platform: "e2e-test".into(),
        memo: "fast and accurate".into(),
    });
    assert!(state.apply_tx(&tx, 0).is_err(), "ReputationAttest is deprecated and must be rejected");
    assert_eq!(state.reputation.len(), 0);

    // === Verify final state ===
    let root = state.state_root();
    assert_ne!(root, [0u8; 32]);

    // Gas accounting: addr1 used 5 successful txs (rep attest failed, no gas charged)
    // addr1: 10B - 2B transfer - 5 * GAS_FEE
    assert_eq!(state.get_balance(&addr1), 10_000_000_000 - 2_000_000_000 - 5 * GAS_FEE);
    // addr2: 10B + 2B transfer - 1 * GAS_FEE (agent register only; rep attest failed)
    assert_eq!(state.get_balance(&addr2), 10_000_000_000 + 2_000_000_000 - 1 * GAS_FEE);

    println!("\n=== E2E Test Summary ===");
    println!("Transactions: 7 (2 register + 1 CLAW transfer + 1 token create + 1 custom transfer + 1 service; 2 rep attests rejected)");
    println!("Agents: {}", state.agents.len());
    println!("Custom tokens: {}", state.tokens.len());
    println!("Reputation records: {}", state.reputation.len());
    println!("Services: {}", state.services.len());
    println!("State root: {}", hex::encode(root));
    println!("Final balance addr1: {} base units", state.get_balance(&addr1));
    println!("Final balance addr2: {} base units", state.get_balance(&addr2));
}

#[test]
fn storage_block_persistence() {
    use claw_storage::ChainStore;
    use claw_types::block::Block;

    let dir = tempfile::tempdir().unwrap();
    let store = ChainStore::open(dir.path().join("test.redb")).unwrap();

    // Create and store genesis
    let genesis_state = claw_state::WorldState::default();
    let mut genesis = Block {
        height: 0,
        prev_hash: [0u8; 32],
        timestamp: 1710000000,
        validator: [0u8; 32],
        transactions: vec![],
        state_root: genesis_state.state_root(),
        hash: [0u8; 32],
        signatures: Vec::new(),
        events: Vec::new(),
    };
    genesis.hash = genesis.compute_hash();
    store.put_block(&genesis).unwrap();

    // Store a block with transactions
    let (sk, _) = generate_keypair();
    let tx = make_tx(&sk, 1, TxType::AgentRegister, &AgentRegisterPayload {
        name: "stored-agent".into(),
        metadata: BTreeMap::new(),
    });
    let tx_hash = tx.hash();

    let mut block1 = Block {
        height: 1,
        prev_hash: genesis.hash,
        timestamp: 1710000003,
        validator: [1u8; 32],
        transactions: vec![tx],
        state_root: [99u8; 32],
        hash: [0u8; 32],
        signatures: Vec::new(),
        events: Vec::new(),
    };
    block1.hash = block1.compute_hash();
    store.put_block(&block1).unwrap();

    // Verify persistence
    assert_eq!(store.get_latest_height().unwrap(), Some(1));

    let loaded = store.get_block(1).unwrap().unwrap();
    assert_eq!(loaded.height, 1);
    assert_eq!(loaded.transactions.len(), 1);
    assert!(loaded.verify_hash());

    // TX index works
    assert_eq!(store.get_tx_block_height(&tx_hash).unwrap(), Some(1));
}
