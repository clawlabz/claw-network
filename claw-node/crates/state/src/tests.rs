//! WorldState tests.

#[cfg(test)]
mod tests {
    use crate::error::StateError;
    use crate::world::WorldState;
    use claw_crypto::ed25519_dalek::SigningKey;
    use claw_crypto::keys::generate_keypair;
    use claw_crypto::signer::sign_transaction;
    use claw_types::state::*;
    use claw_types::transaction::*;
    use std::collections::BTreeMap;

    /// Helper: create a funded WorldState with one keypair.
    fn setup() -> (WorldState, SigningKey, [u8; 32]) {
        let (sk, vk) = generate_keypair();
        let addr = vk.to_bytes();
        let mut state = WorldState::default();
        state.balances.insert(addr, 100 * GAS_FEE + 1_000_000_000); // plenty of CLAW
        (state, sk, addr)
    }

    /// Helper: build and sign a transaction.
    fn make_tx(
        sk: &SigningKey,
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

    // === Agent Register ===

    #[test]
    fn agent_register_success() {
        let (mut state, sk, addr) = setup();
        let payload = AgentRegisterPayload {
            name: "test-agent".into(),
            metadata: BTreeMap::new(),
        };
        let tx = make_tx(&sk, 1, TxType::AgentRegister, &payload);
        assert!(state.apply_tx(&tx, 0).is_ok());
        assert_eq!(state.agents.get(&addr).unwrap().name, "test-agent");
        assert_eq!(state.get_nonce(&addr), 1);
    }

    #[test]
    fn agent_register_duplicate_fails() {
        let (mut state, sk, _) = setup();
        let payload = AgentRegisterPayload {
            name: "agent".into(),
            metadata: BTreeMap::new(),
        };
        let tx1 = make_tx(&sk, 1, TxType::AgentRegister, &payload);
        state.apply_tx(&tx1, 0).unwrap();

        let tx2 = make_tx(&sk, 2, TxType::AgentRegister, &payload);
        assert_eq!(
            state.apply_tx(&tx2, 0).map(|(_, _)| ()),
            Err(StateError::AgentAlreadyRegistered)
        );
    }

    #[test]
    fn agent_register_empty_name_fails() {
        let (mut state, sk, _) = setup();
        let payload = AgentRegisterPayload {
            name: "".into(),
            metadata: BTreeMap::new(),
        };
        let tx = make_tx(&sk, 1, TxType::AgentRegister, &payload);
        assert!(matches!(state.apply_tx(&tx, 0), Err(StateError::NameTooLong { .. })));
    }

    // === Token Transfer ===

    #[test]
    fn token_transfer_success() {
        let (mut state, sk, addr) = setup();
        let (_, vk2) = generate_keypair();
        let addr2 = vk2.to_bytes();

        let payload = TokenTransferPayload {
            to: addr2,
            amount: 500_000_000,
        };
        let tx = make_tx(&sk, 1, TxType::TokenTransfer, &payload);
        let initial = state.get_balance(&addr);
        state.apply_tx(&tx, 0).unwrap();

        assert_eq!(state.get_balance(&addr2), 500_000_000);
        assert_eq!(
            state.get_balance(&addr),
            initial - 500_000_000 - GAS_FEE
        );
    }

    #[test]
    fn token_transfer_insufficient_balance() {
        let (mut state, sk, _) = setup();
        let (_, vk2) = generate_keypair();
        let payload = TokenTransferPayload {
            to: vk2.to_bytes(),
            amount: u128::MAX,
        };
        let tx = make_tx(&sk, 1, TxType::TokenTransfer, &payload);
        assert!(matches!(
            state.apply_tx(&tx, 0),
            Err(StateError::InsufficientBalance { .. })
        ));
    }

    #[test]
    fn token_transfer_zero_amount() {
        let (mut state, sk, _) = setup();
        let (_, vk2) = generate_keypair();
        let payload = TokenTransferPayload {
            to: vk2.to_bytes(),
            amount: 0,
        };
        let tx = make_tx(&sk, 1, TxType::TokenTransfer, &payload);
        assert_eq!(state.apply_tx(&tx, 0).map(|(_, _)| ()), Err(StateError::ZeroAmount));
    }

    // === Nonce ===

    #[test]
    fn wrong_nonce_rejected() {
        let (mut state, sk, _) = setup();
        let payload = AgentRegisterPayload {
            name: "agent".into(),
            metadata: BTreeMap::new(),
        };
        let tx = make_tx(&sk, 5, TxType::AgentRegister, &payload); // should be 1
        assert_eq!(
            state.apply_tx(&tx, 0).map(|(_, _)| ()),
            Err(StateError::InvalidNonce {
                expected: 1,
                got: 5,
            })
        );
    }

    // === Gas ===

    #[test]
    fn insufficient_gas_rejected() {
        let (sk, vk) = generate_keypair();
        let addr = vk.to_bytes();
        let mut state = WorldState::default();
        state.balances.insert(addr, GAS_FEE - 1); // not enough for gas

        // Use ServiceRegister (charges gas) instead of AgentRegister (gas-free)
        let payload = ServiceRegisterPayload {
            service_type: "test".into(),
            description: "test".into(),
            endpoint: "http://test".into(),
            price_token: [0u8; 32],
            price_amount: 0,
            active: true,
        };
        let tx = make_tx(&sk, 1, TxType::ServiceRegister, &payload);
        assert!(matches!(
            state.apply_tx(&tx, 0),
            Err(StateError::InsufficientBalance { .. })
        ));
    }

    #[test]
    fn gas_burned_on_success() {
        let (mut state, sk, addr) = setup();
        // Register agent first (gas-free)
        let reg = AgentRegisterPayload { name: "agent".into(), metadata: BTreeMap::new() };
        state.apply_tx(&make_tx(&sk, 1, TxType::AgentRegister, &reg), 0).unwrap();
        let balance_after_reg = state.get_balance(&addr);

        // ServiceRegister charges gas
        let payload = ServiceRegisterPayload {
            service_type: "test".into(),
            description: "test".into(),
            endpoint: "http://test".into(),
            price_token: [0u8; 32],
            price_amount: 0,
            active: true,
        };
        let tx = make_tx(&sk, 2, TxType::ServiceRegister, &payload);
        state.apply_tx(&tx, 0).unwrap();
        assert_eq!(state.get_balance(&addr), balance_after_reg - GAS_FEE);
    }

    #[test]
    fn gas_refunded_on_failure() {
        let (mut state, sk, addr) = setup();
        // Register first
        let payload = AgentRegisterPayload {
            name: "agent".into(),
            metadata: BTreeMap::new(),
        };
        let tx1 = make_tx(&sk, 1, TxType::AgentRegister, &payload);
        state.apply_tx(&tx1, 0).unwrap();

        let balance_after_first = state.get_balance(&addr);

        // Try duplicate register — should fail, gas refunded
        let tx2 = make_tx(&sk, 2, TxType::AgentRegister, &payload);
        assert!(state.apply_tx(&tx2, 0).is_err());
        assert_eq!(state.get_balance(&addr), balance_after_first);
    }

    // === Token Create ===

    #[test]
    fn token_create_success() {
        let (mut state, sk, addr) = setup();
        // Must be registered agent
        let reg = AgentRegisterPayload {
            name: "agent".into(),
            metadata: BTreeMap::new(),
        };
        state.apply_tx(&make_tx(&sk, 1, TxType::AgentRegister, &reg), 0).unwrap();

        let payload = TokenCreatePayload {
            name: "TestCoin".into(),
            symbol: "TST".into(),
            decimals: 6,
            total_supply: 1_000_000,
        };
        let tx = make_tx(&sk, 2, TxType::TokenCreate, &payload);
        state.apply_tx(&tx, 0).unwrap();

        // Find the created token
        assert_eq!(state.tokens.len(), 1);
        let (token_id, token_def) = state.tokens.iter().next().unwrap();
        assert_eq!(token_def.name, "TestCoin");
        assert_eq!(token_def.issuer, addr);
        assert_eq!(
            state.get_token_balance(&addr, token_id),
            1_000_000
        );
    }

    #[test]
    fn token_create_not_registered_fails() {
        let (mut state, sk, _) = setup();
        let payload = TokenCreatePayload {
            name: "TestCoin".into(),
            symbol: "TST".into(),
            decimals: 6,
            total_supply: 1_000_000,
        };
        let tx = make_tx(&sk, 1, TxType::TokenCreate, &payload);
        assert_eq!(state.apply_tx(&tx, 0).map(|(_, _)| ()), Err(StateError::AgentNotRegistered));
    }

    // === Token Mint Transfer ===

    #[test]
    fn token_mint_transfer_success() {
        let (mut state, sk, addr) = setup();
        let (_, vk2) = generate_keypair();
        let addr2 = vk2.to_bytes();

        // Register + create token
        let reg = AgentRegisterPayload { name: "a".into(), metadata: BTreeMap::new() };
        state.apply_tx(&make_tx(&sk, 1, TxType::AgentRegister, &reg), 0).unwrap();
        let create = TokenCreatePayload {
            name: "Coin".into(), symbol: "C".into(), decimals: 0, total_supply: 100,
        };
        state.apply_tx(&make_tx(&sk, 2, TxType::TokenCreate, &create), 0).unwrap();
        let token_id = *state.tokens.keys().next().unwrap();

        // Transfer custom token
        let payload = TokenMintTransferPayload { token_id, to: addr2, amount: 30 };
        state.apply_tx(&make_tx(&sk, 3, TxType::TokenMintTransfer, &payload), 0).unwrap();

        assert_eq!(state.get_token_balance(&addr, &token_id), 70);
        assert_eq!(state.get_token_balance(&addr2, &token_id), 30);
    }

    #[test]
    fn token_mint_transfer_native_id_fails() {
        let (mut state, sk, _) = setup();
        let reg = AgentRegisterPayload { name: "a".into(), metadata: BTreeMap::new() };
        state.apply_tx(&make_tx(&sk, 1, TxType::AgentRegister, &reg), 0).unwrap();

        let payload = TokenMintTransferPayload {
            token_id: NATIVE_TOKEN_ID,
            to: [2u8; 32],
            amount: 10,
        };
        let tx = make_tx(&sk, 2, TxType::TokenMintTransfer, &payload);
        assert_eq!(state.apply_tx(&tx, 0).map(|(_, _)| ()), Err(StateError::NativeTokenIdForCustom));
    }

    // === Reputation Attest ===

    #[test]
    fn reputation_attest_success() {
        let (mut state, sk1, _) = setup();
        let (sk2, vk2) = generate_keypair();
        let addr2 = vk2.to_bytes();
        state.balances.insert(addr2, 100 * GAS_FEE);

        // Register both
        let reg1 = AgentRegisterPayload { name: "a1".into(), metadata: BTreeMap::new() };
        state.apply_tx(&make_tx(&sk1, 1, TxType::AgentRegister, &reg1), 0).unwrap();
        let reg2 = AgentRegisterPayload { name: "a2".into(), metadata: BTreeMap::new() };
        state.apply_tx(&make_tx(&sk2, 1, TxType::AgentRegister, &reg2), 0).unwrap();

        let payload = ReputationAttestPayload {
            to: addr2,
            category: "game".into(),
            score: 80,
            platform: "arena".into(),
            memo: "good player".into(),
        };
        // ReputationAttest is deprecated — all submissions are rejected.
        let result = state.apply_tx(&make_tx(&sk1, 2, TxType::ReputationAttest, &payload), 0);
        assert!(result.is_err(), "deprecated ReputationAttest must be rejected");
        assert_eq!(state.reputation.len(), 0);
    }

    #[test]
    fn reputation_self_attest_fails() {
        let (mut state, sk, addr) = setup();
        let reg = AgentRegisterPayload { name: "a".into(), metadata: BTreeMap::new() };
        state.apply_tx(&make_tx(&sk, 1, TxType::AgentRegister, &reg), 0).unwrap();

        let payload = ReputationAttestPayload {
            to: addr,
            category: "game".into(),
            score: 100,
            platform: "arena".into(),
            memo: "".into(),
        };
        let tx = make_tx(&sk, 2, TxType::ReputationAttest, &payload);
        // ReputationAttest is deprecated — rejected before self-attest check.
        assert!(state.apply_tx(&tx, 0).is_err());
    }

    #[test]
    fn reputation_score_out_of_range() {
        let (mut state, sk1, _) = setup();
        let (sk2, vk2) = generate_keypair();
        let addr2 = vk2.to_bytes();
        state.balances.insert(addr2, 100 * GAS_FEE);

        let reg1 = AgentRegisterPayload { name: "a1".into(), metadata: BTreeMap::new() };
        state.apply_tx(&make_tx(&sk1, 1, TxType::AgentRegister, &reg1), 0).unwrap();
        let reg2 = AgentRegisterPayload { name: "a2".into(), metadata: BTreeMap::new() };
        state.apply_tx(&make_tx(&sk2, 1, TxType::AgentRegister, &reg2), 0).unwrap();

        let payload = ReputationAttestPayload {
            to: addr2, category: "x".into(), score: 101, platform: "p".into(), memo: "".into(),
        };
        // ReputationAttest is deprecated — rejected before score validation.
        assert!(
            state.apply_tx(&make_tx(&sk1, 2, TxType::ReputationAttest, &payload), 0).is_err()
        );
    }

    // === Service Register ===

    #[test]
    fn service_register_success() {
        let (mut state, sk, addr) = setup();
        let reg = AgentRegisterPayload { name: "a".into(), metadata: BTreeMap::new() };
        state.apply_tx(&make_tx(&sk, 1, TxType::AgentRegister, &reg), 0).unwrap();

        let payload = ServiceRegisterPayload {
            service_type: "translation".into(),
            description: "EN-CN".into(),
            price_token: NATIVE_TOKEN_ID,
            price_amount: 10_000_000,
            endpoint: "https://example.com/translate".into(),
            active: true,
        };
        state.apply_tx(&make_tx(&sk, 2, TxType::ServiceRegister, &payload), 0).unwrap();

        let svc = state.services.get(&(addr, "translation".to_string())).unwrap();
        assert_eq!(svc.endpoint, "https://example.com/translate");
        assert!(svc.active);
    }

    #[test]
    fn service_register_upsert() {
        let (mut state, sk, addr) = setup();
        let reg = AgentRegisterPayload { name: "a".into(), metadata: BTreeMap::new() };
        state.apply_tx(&make_tx(&sk, 1, TxType::AgentRegister, &reg), 0).unwrap();

        let payload1 = ServiceRegisterPayload {
            service_type: "review".into(),
            description: "v1".into(),
            price_token: NATIVE_TOKEN_ID,
            price_amount: 100,
            endpoint: "https://v1.com".into(),
            active: true,
        };
        state.apply_tx(&make_tx(&sk, 2, TxType::ServiceRegister, &payload1), 0).unwrap();

        // Update same service type
        let payload2 = ServiceRegisterPayload {
            service_type: "review".into(),
            description: "v2".into(),
            price_token: NATIVE_TOKEN_ID,
            price_amount: 200,
            endpoint: "https://v2.com".into(),
            active: false,
        };
        state.apply_tx(&make_tx(&sk, 3, TxType::ServiceRegister, &payload2), 0).unwrap();

        let svc = state.services.get(&(addr, "review".to_string())).unwrap();
        assert_eq!(svc.description, "v2");
        assert!(!svc.active);
        assert_eq!(state.services.len(), 1); // upsert, not insert
    }

    // === State Root ===

    #[test]
    fn state_root_deterministic() {
        let (mut state, sk, _) = setup();
        let payload = AgentRegisterPayload { name: "a".into(), metadata: BTreeMap::new() };
        state.apply_tx(&make_tx(&sk, 1, TxType::AgentRegister, &payload), 0).unwrap();

        let root1 = state.state_root();
        let root2 = state.state_root();
        assert_eq!(root1, root2);
        assert_ne!(root1, [0u8; 32]);
    }

    #[test]
    fn state_root_changes_on_mutation() {
        let (mut state, sk, _) = setup();
        let root_before = state.state_root();

        let payload = AgentRegisterPayload { name: "a".into(), metadata: BTreeMap::new() };
        state.apply_tx(&make_tx(&sk, 1, TxType::AgentRegister, &payload), 0).unwrap();

        let root_after = state.state_root();
        assert_ne!(root_before, root_after);
    }

    // === Signature ===

    #[test]
    fn invalid_signature_rejected() {
        let (mut state, sk, _) = setup();
        let payload = AgentRegisterPayload { name: "a".into(), metadata: BTreeMap::new() };
        let mut tx = make_tx(&sk, 1, TxType::AgentRegister, &payload);
        tx.signature[0] ^= 0xFF; // corrupt signature
        assert_eq!(state.apply_tx(&tx, 0).map(|(_, _)| ()), Err(StateError::InvalidSignature));
    }

    // === PlatformActivityReport ===

    /// Helper: set up a Platform Agent (registered + staked >= 50k CLAW).
    fn setup_platform_agent() -> (WorldState, claw_crypto::ed25519_dalek::SigningKey, [u8; 32],
                                  claw_crypto::ed25519_dalek::SigningKey, [u8; 32]) {
        let (sk1, vk1) = generate_keypair();
        let addr1 = vk1.to_bytes();
        let (sk2, vk2) = generate_keypair();
        let addr2 = vk2.to_bytes();
        let mut state = WorldState::default();

        // Fund both agents generously
        state.balances.insert(addr1, 100_000_000_000_000); // 100k CLAW
        state.balances.insert(addr2, 100_000_000_000_000);

        // Register both
        let reg1 = AgentRegisterPayload { name: "platform1".into(), metadata: BTreeMap::new() };
        state.apply_tx(&make_tx(&sk1, 1, TxType::AgentRegister, &reg1), 0).unwrap();
        let reg2 = AgentRegisterPayload { name: "agent2".into(), metadata: BTreeMap::new() };
        state.apply_tx(&make_tx(&sk2, 1, TxType::AgentRegister, &reg2), 0).unwrap();

        // Stake 50k CLAW for addr1 (Platform Agent threshold)
        let stake = claw_types::transaction::StakeDepositPayload { amount: 50_000_000_000_000, validator: [0u8; 32], commission_bps: 10000 };
        state.apply_tx(&make_tx(&sk1, 2, TxType::StakeDeposit, &stake), 0).unwrap();

        (state, sk1, addr1, sk2, addr2)
    }

    #[test]
    fn platform_activity_report_success() {
        let (mut state, sk1, _addr1, _sk2, addr2) = setup_platform_agent();

        let payload = PlatformActivityReportPayload {
            reports: vec![ActivityEntry {
                agent: addr2,
                action_count: 42,
                action_type: "game_played".into(),
            }],
        };
        let tx = make_tx(&sk1, 3, TxType::PlatformActivityReport, &payload);
        state.apply_tx(&tx, 0).unwrap();

        // Check platform activity was recorded
        let agg = state.platform_activity.get(&addr2).unwrap();
        assert_eq!(agg.total_actions, 42);
        assert_eq!(agg.platform_count, 1);
    }

    #[test]
    fn platform_activity_report_insufficient_stake() {
        let (mut state, _sk1, _addr1, sk2, addr2) = setup_platform_agent();

        // addr2 is registered but not staked enough
        let payload = PlatformActivityReportPayload {
            reports: vec![ActivityEntry {
                agent: addr2,
                action_count: 10,
                action_type: "task_completed".into(),
            }],
        };
        let tx = make_tx(&sk2, 2, TxType::PlatformActivityReport, &payload);
        assert!(matches!(
            state.apply_tx(&tx, 0),
            Err(StateError::PlatformStakeTooLow { .. })
        ));
    }

    #[test]
    fn platform_activity_report_duplicate_epoch() {
        let (mut state, sk1, _addr1, _sk2, addr2) = setup_platform_agent();

        let payload = PlatformActivityReportPayload {
            reports: vec![ActivityEntry {
                agent: addr2,
                action_count: 10,
                action_type: "game_played".into(),
            }],
        };

        // First report succeeds
        state.apply_tx(&make_tx(&sk1, 3, TxType::PlatformActivityReport, &payload), 0).unwrap();

        // Second report in same epoch fails
        let tx2 = make_tx(&sk1, 4, TxType::PlatformActivityReport, &payload);
        assert_eq!(
            state.apply_tx(&tx2, 0).map(|(_, _)| ()),
            Err(StateError::PlatformReportAlreadySubmitted)
        );
    }

    #[test]
    fn platform_activity_report_action_type_too_long() {
        let (mut state, sk1, _addr1, _sk2, addr2) = setup_platform_agent();

        let payload = PlatformActivityReportPayload {
            reports: vec![ActivityEntry {
                agent: addr2,
                action_count: 1,
                action_type: "x".repeat(65), // exceeds 64-byte limit
            }],
        };
        let tx = make_tx(&sk1, 3, TxType::PlatformActivityReport, &payload);
        assert!(matches!(
            state.apply_tx(&tx, 0),
            Err(StateError::ActionTypeTooLong { .. })
        ));
    }

    #[test]
    fn platform_activity_report_unregistered_target() {
        let (mut state, sk1, _addr1, _sk2, _addr2) = setup_platform_agent();

        let payload = PlatformActivityReportPayload {
            reports: vec![ActivityEntry {
                agent: [99u8; 32], // not a registered agent
                action_count: 5,
                action_type: "query_served".into(),
            }],
        };
        let tx = make_tx(&sk1, 3, TxType::PlatformActivityReport, &payload);
        assert_eq!(
            state.apply_tx(&tx, 0).map(|(_, _)| ()),
            Err(StateError::AgentNotRegistered)
        );
    }

    #[test]
    fn platform_activity_report_action_count_exceeds_max_is_rejected() {
        let (mut state, sk1, _addr1, _sk2, addr2) = setup_platform_agent();

        let payload = PlatformActivityReportPayload {
            reports: vec![ActivityEntry {
                agent: addr2,
                action_count: 10_001, // one over the 10,000 limit
                action_type: "game_played".into(),
            }],
        };
        let tx = make_tx(&sk1, 3, TxType::PlatformActivityReport, &payload);
        assert!(matches!(
            state.apply_tx(&tx, 0),
            Err(StateError::ActionCountTooHigh { .. })
        ));
    }

    #[test]
    fn platform_activity_report_action_count_at_max_is_accepted() {
        let (mut state, sk1, _addr1, _sk2, addr2) = setup_platform_agent();

        let payload = PlatformActivityReportPayload {
            reports: vec![ActivityEntry {
                agent: addr2,
                action_count: 10_000, // exactly at the limit
                action_type: "game_played".into(),
            }],
        };
        let tx = make_tx(&sk1, 3, TxType::PlatformActivityReport, &payload);
        state.apply_tx(&tx, 0).unwrap();

        let agg = state.platform_activity.get(&addr2).unwrap();
        assert_eq!(agg.total_actions, 10_000);
    }

    #[test]
    fn platform_activity_report_action_count_zero_is_accepted() {
        let (mut state, sk1, _addr1, _sk2, addr2) = setup_platform_agent();

        let payload = PlatformActivityReportPayload {
            reports: vec![ActivityEntry {
                agent: addr2,
                action_count: 0, // zero is a valid edge case
                action_type: "game_played".into(),
            }],
        };
        let tx = make_tx(&sk1, 3, TxType::PlatformActivityReport, &payload);
        state.apply_tx(&tx, 0).unwrap();

        let agg = state.platform_activity.get(&addr2).unwrap();
        assert_eq!(agg.total_actions, 0);
        assert_eq!(agg.platform_count, 1);
    }

    // === Activity Stats Tracking ===

    #[test]
    fn activity_stats_updated_on_tx() {
        let (mut state, sk, addr) = setup();

        // Register agent (should increment tx_count)
        let reg = AgentRegisterPayload { name: "agent".into(), metadata: BTreeMap::new() };
        state.apply_tx(&make_tx(&sk, 1, TxType::AgentRegister, &reg), 0).unwrap();

        let stats = state.activity_stats.get(&addr).unwrap();

        assert_eq!(stats.tx_count, 1);
        // AgentRegister is gas-free, so gas_consumed is 0 after first tx
        assert_eq!(stats.gas_consumed, 0);

        // Token create (should also increment tokens_created, and charges gas)
        let create = TokenCreatePayload {
            name: "Test".into(),
            symbol: "T".into(),
            decimals: 0,
            total_supply: 100,
        };
        state.apply_tx(&make_tx(&sk, 2, TxType::TokenCreate, &create), 0).unwrap();

        let stats = state.activity_stats.get(&addr).unwrap();
        assert_eq!(stats.tx_count, 2);
        assert_eq!(stats.tokens_created, 1);
    }

    // === Miner Register ===

    #[test]
    fn test_miner_register_success() {
        let (mut state, sk, addr) = setup();
        let payload = MinerRegisterPayload {
            tier: 1,
            ip_addr: vec![192, 168, 1, 10],
            name: "my-miner".into(),
        };
        let tx = make_tx(&sk, 1, TxType::MinerRegister, &payload);
        state.apply_tx(&tx, 0).unwrap();

        let miner = state.miners.get(&addr).expect("miner should be registered");
        assert_eq!(miner.name, "my-miner");
        assert_eq!(miner.tier, MinerTier::Online);
        assert!(miner.active);
        assert_eq!(miner.reputation_bps, REPUTATION_NEWCOMER_BPS);
        assert_eq!(miner.ip_prefix, vec![192, 168, 1]);
    }

    #[test]
    fn test_miner_register_duplicate() {
        let (mut state, sk, _) = setup();
        let payload = MinerRegisterPayload {
            tier: 1,
            ip_addr: vec![10, 0, 0, 1],
            name: "miner1".into(),
        };
        state.apply_tx(&make_tx(&sk, 1, TxType::MinerRegister, &payload), 0).unwrap();

        let tx2 = make_tx(&sk, 2, TxType::MinerRegister, &payload);
        assert_eq!(state.apply_tx(&tx2, 0).map(|(_, _)| ()), Err(StateError::MinerAlreadyRegistered));
    }

    #[test]
    fn test_miner_register_invalid_tier() {
        let (mut state, sk, _) = setup();
        let payload = MinerRegisterPayload {
            tier: 5, // invalid
            ip_addr: vec![10, 0, 0, 1],
            name: "miner".into(),
        };
        let tx = make_tx(&sk, 1, TxType::MinerRegister, &payload);
        assert_eq!(state.apply_tx(&tx, 0).map(|(_, _)| ()), Err(StateError::InvalidMinerTier(5)));
    }

    #[test]
    fn test_miner_register_subnet_limit() {
        // Register 3 miners on same /24 subnet, then a 4th should fail
        let mut state = WorldState::default();
        let mut keys = Vec::new();
        for i in 0..4u8 {
            let sk = SigningKey::from_bytes(&[10 + i; 32]);
            let addr = claw_crypto::ed25519_dalek::VerifyingKey::from(&sk).to_bytes();
            state.balances.insert(addr, 100 * GAS_FEE + 1_000_000_000);
            keys.push((sk, addr));
        }

        for i in 0..3 {
            let payload = MinerRegisterPayload {
                tier: 1,
                ip_addr: vec![10, 0, 0, (i + 1) as u8], // same /24: 10.0.0.x
                name: format!("miner-{}", i),
            };
            let tx = make_tx(&keys[i].0, 1, TxType::MinerRegister, &payload);
            state.apply_tx(&tx, 0).unwrap();
        }

        // 4th miner on same subnet should fail
        let payload = MinerRegisterPayload {
            tier: 1,
            ip_addr: vec![10, 0, 0, 99],
            name: "miner-overflow".into(),
        };
        let tx = make_tx(&keys[3].0, 1, TxType::MinerRegister, &payload);
        assert_eq!(
            state.apply_tx(&tx, 0).map(|(_, _)| ()),
            Err(StateError::SubnetLimitReached { max: MAX_MINERS_PER_SUBNET })
        );
    }

    #[test]
    fn test_miner_register_invalid_ip() {
        let (mut state, sk, _) = setup();
        let payload = MinerRegisterPayload {
            tier: 1,
            ip_addr: vec![1, 2], // bad length
            name: "miner".into(),
        };
        let tx = make_tx(&sk, 1, TxType::MinerRegister, &payload);
        assert_eq!(state.apply_tx(&tx, 0).map(|(_, _)| ()), Err(StateError::InvalidIpLength(2)));
    }

    // === Miner Heartbeat ===

    /// Helper: register a miner and return the state + signing key + address.
    fn setup_miner() -> (WorldState, SigningKey, [u8; 32]) {
        let (mut state, sk, addr) = setup();
        let payload = MinerRegisterPayload {
            tier: 1,
            ip_addr: vec![10, 0, 0, 1],
            name: "test-miner".into(),
        };
        state.apply_tx(&make_tx(&sk, 1, TxType::MinerRegister, &payload), 0).unwrap();
        (state, sk, addr)
    }

    #[test]
    fn test_miner_heartbeat_success() {
        let (mut state, sk, addr) = setup_miner();
        // Advance block_height past the heartbeat interval
        state.block_height = MINER_HEARTBEAT_INTERVAL_V1 + 1;

        let payload = MinerHeartbeatPayload {
            latest_block_hash: [0u8; 32],
            latest_height: state.block_height,
        };
        let tx = make_tx(&sk, 2, TxType::MinerHeartbeat, &payload);
        state.apply_tx(&tx, 0).unwrap();

        let miner = state.miners.get(&addr).unwrap();
        assert_eq!(miner.last_heartbeat, MINER_HEARTBEAT_INTERVAL_V1 + 1);
        assert!(miner.active);
    }

    #[test]
    fn test_miner_heartbeat_not_registered() {
        let (mut state, sk, _) = setup();
        state.block_height = 5000;
        let payload = MinerHeartbeatPayload {
            latest_block_hash: [0u8; 32],
            latest_height: 5000,
        };
        let tx = make_tx(&sk, 1, TxType::MinerHeartbeat, &payload);
        assert_eq!(state.apply_tx(&tx, 0).map(|(_, _)| ()), Err(StateError::MinerNotRegistered));
    }

    #[test]
    fn test_miner_heartbeat_too_early() {
        let (mut state, sk, _) = setup_miner();
        // Don't advance block_height past interval (miner registered at height 0)
        state.block_height = 500; // < MINER_HEARTBEAT_INTERVAL_V1

        let payload = MinerHeartbeatPayload {
            latest_block_hash: [0u8; 32],
            latest_height: 500,
        };
        let tx = make_tx(&sk, 2, TxType::MinerHeartbeat, &payload);
        assert!(matches!(
            state.apply_tx(&tx, 0),
            Err(StateError::HeartbeatTooEarly { .. })
        ));
    }

    #[test]
    fn test_miner_heartbeat_gas_free() {
        let (mut state, sk, addr) = setup_miner();
        let balance_before = state.get_balance(&addr);
        state.block_height = MINER_HEARTBEAT_INTERVAL_V1 + 1;

        let payload = MinerHeartbeatPayload {
            latest_block_hash: [0u8; 32],
            latest_height: state.block_height,
        };
        let tx = make_tx(&sk, 2, TxType::MinerHeartbeat, &payload);
        let (fee, _) = state.apply_tx(&tx, 0).unwrap();

        // Gas should be 0
        assert_eq!(fee, 0);
        // Balance should be unchanged (no gas deducted)
        assert_eq!(state.get_balance(&addr), balance_before);
    }

    #[test]
    fn test_miner_heartbeat_updates_reputation_v1() {
        // V1 heartbeat handler upgrades reputation based on miner age.
        // Only tests newcomer stage since established/veteran heights exceed
        // HEARTBEAT_V2_HEIGHT and use a different code path (epoch boundary).
        let (mut state, sk, addr) = setup_miner();

        state.block_height = MINER_HEARTBEAT_INTERVAL_V1 + 1;
        let hb = MinerHeartbeatPayload {
            latest_block_hash: [0u8; 32],
            latest_height: state.block_height,
        };
        state.apply_tx(&make_tx(&sk, 2, TxType::MinerHeartbeat, &hb), 0).unwrap();
        assert_eq!(state.miners.get(&addr).unwrap().reputation_bps, REPUTATION_NEWCOMER_BPS);
    }

    #[test]
    fn test_v2_epoch_boundary_upgrades_reputation() {
        // V2: reputation is upgraded at epoch boundary, not in the heartbeat handler.
        use crate::rewards::process_miner_epoch_boundary;

        let (mut state, addr, epoch_start) = setup_v2_miner();
        let height = epoch_start + MINER_EPOCH_LENGTH;

        // Set registered_at so age >= BLOCKS_7_DAYS (established tier)
        state.miners.get_mut(&addr).unwrap().registered_at = height - BLOCKS_7_DAYS - 1;
        state.miners.get_mut(&addr).unwrap().reputation_bps = REPUTATION_NEWCOMER_BPS;

        // Heartbeat in the settled epoch
        let settled_epoch_start = height - MINER_EPOCH_LENGTH;
        state.miners.get_mut(&addr).unwrap().last_heartbeat = settled_epoch_start + 50;

        state.block_height = height;
        process_miner_epoch_boundary(&mut state, height);

        // Should be upgraded from newcomer to at least established
        assert!(state.miners.get(&addr).unwrap().reputation_bps >= REPUTATION_ESTABLISHED_BPS);
    }

    // === Reward System (Mining Upgrade) ===

    #[test]
    fn test_reward_per_block_new_schedule() {
        use crate::rewards::{reward_per_block, MINING_UPGRADE_HEIGHT, HALVING_PERIOD};

        // Before upgrade: legacy schedule
        assert_eq!(reward_per_block(0), 10_000_000_000);
        assert_eq!(reward_per_block(MINING_UPGRADE_HEIGHT - 1), 10_000_000_000);

        // After upgrade: geometric halving from 8 CLAW
        assert_eq!(reward_per_block(MINING_UPGRADE_HEIGHT), 8_000_000_000);
        assert_eq!(reward_per_block(MINING_UPGRADE_HEIGHT + HALVING_PERIOD - 1), 8_000_000_000);
        assert_eq!(reward_per_block(MINING_UPGRADE_HEIGHT + HALVING_PERIOD), 4_000_000_000);
        assert_eq!(reward_per_block(MINING_UPGRADE_HEIGHT + 2 * HALVING_PERIOD), 2_000_000_000);
        assert_eq!(reward_per_block(MINING_UPGRADE_HEIGHT + 3 * HALVING_PERIOD), 1_000_000_000);
        assert_eq!(reward_per_block(MINING_UPGRADE_HEIGHT + 4 * HALVING_PERIOD), 500_000_000);
        assert_eq!(reward_per_block(MINING_UPGRADE_HEIGHT + 5 * HALVING_PERIOD), 250_000_000);
        // Beyond 6th period: floor at 250M (0.25 CLAW)
        assert_eq!(reward_per_block(MINING_UPGRADE_HEIGHT + 6 * HALVING_PERIOD), 250_000_000);
    }

    #[test]
    fn test_reward_per_block_upgrade_transition() {
        use crate::rewards::{reward_per_block, MINING_UPGRADE_HEIGHT};

        // Right before upgrade: legacy schedule
        let before = reward_per_block(MINING_UPGRADE_HEIGHT - 1);
        assert_eq!(before, 10_000_000_000); // Year 1 legacy

        // Right at upgrade: new schedule starts
        let at = reward_per_block(MINING_UPGRADE_HEIGHT);
        assert_eq!(at, 8_000_000_000); // First period of new schedule
    }

    #[test]
    fn test_distribute_mining_rewards_basic() {
        use crate::rewards::{distribute_mining_rewards, MINING_UPGRADE_HEIGHT, genesis_address_pub, NODE_INCENTIVE_POOL_INDEX};

        let mut state = WorldState::default();
        let pool_addr = genesis_address_pub(NODE_INCENTIVE_POOL_INDEX);
        state.balances.insert(pool_addr, 1_000_000_000_000); // plenty

        // Create two miners with different weights (tier_weight=1 for both, different reputation)
        let addr1 = [1u8; 32];
        let addr2 = [2u8; 32];
        state.miners.insert(addr1, MinerInfo {
            address: addr1,
            tier: MinerTier::Online,
            name: "m1".into(),
            registered_at: 0,
            last_heartbeat: MINING_UPGRADE_HEIGHT,
            ip_prefix: vec![10, 0, 0],
            active: true,
            reputation_bps: 10_000, // 1.0x
            pending_rewards: 0,
            pending_epoch: 0,
            epoch_attendance: 0,
            consecutive_misses: 0,
        });
        state.miners.insert(addr2, MinerInfo {
            address: addr2,
            tier: MinerTier::Online,
            name: "m2".into(),
            registered_at: 0,
            last_heartbeat: MINING_UPGRADE_HEIGHT,
            ip_prefix: vec![10, 0, 1],
            active: true,
            reputation_bps: 10_000, // 1.0x
            pending_rewards: 0,
            pending_epoch: 0,
            epoch_attendance: 0,
            consecutive_misses: 0,
        });

        let events = distribute_mining_rewards(&mut state, MINING_UPGRADE_HEIGHT);
        assert!(!events.is_empty());

        let m1_bal = state.get_balance(&addr1);
        let m2_bal = state.get_balance(&addr2);
        // Both have equal weight, so they should get equal share
        assert_eq!(m1_bal, m2_bal);
        assert!(m1_bal > 0);
    }

    #[test]
    fn test_distribute_mining_rewards_no_miners() {
        use crate::rewards::{distribute_mining_rewards, MINING_UPGRADE_HEIGHT, genesis_address_pub, NODE_INCENTIVE_POOL_INDEX};

        let mut state = WorldState::default();
        let pool_addr = genesis_address_pub(NODE_INCENTIVE_POOL_INDEX);
        state.balances.insert(pool_addr, 1_000_000_000_000);

        let events = distribute_mining_rewards(&mut state, MINING_UPGRADE_HEIGHT);
        assert!(events.is_empty());
    }

    #[test]
    fn test_distribute_mining_rewards_respects_reputation() {
        use crate::rewards::{distribute_mining_rewards, MINING_UPGRADE_HEIGHT, genesis_address_pub, NODE_INCENTIVE_POOL_INDEX};

        let mut state = WorldState::default();
        let pool_addr = genesis_address_pub(NODE_INCENTIVE_POOL_INDEX);
        state.balances.insert(pool_addr, 1_000_000_000_000);

        let addr1 = [1u8; 32];
        let addr2 = [2u8; 32];
        state.miners.insert(addr1, MinerInfo {
            address: addr1,
            tier: MinerTier::Online,
            name: "veteran".into(),
            registered_at: 0,
            last_heartbeat: MINING_UPGRADE_HEIGHT,
            ip_prefix: vec![10, 0, 0],
            active: true,
            reputation_bps: REPUTATION_VETERAN_BPS, // 10000 = 1.0x
            pending_rewards: 0,
            pending_epoch: 0,
            epoch_attendance: 0,
            consecutive_misses: 0,
        });
        state.miners.insert(addr2, MinerInfo {
            address: addr2,
            tier: MinerTier::Online,
            name: "newcomer".into(),
            registered_at: 0,
            last_heartbeat: MINING_UPGRADE_HEIGHT,
            ip_prefix: vec![10, 0, 1],
            active: true,
            reputation_bps: REPUTATION_NEWCOMER_BPS, // 2000 = 0.2x
            pending_rewards: 0,
            pending_epoch: 0,
            epoch_attendance: 0,
            consecutive_misses: 0,
        });

        distribute_mining_rewards(&mut state, MINING_UPGRADE_HEIGHT);

        let m1_bal = state.get_balance(&addr1);
        let m2_bal = state.get_balance(&addr2);
        // Veteran (10000 weight) should get 5x more than newcomer (2000 weight)
        assert!(m1_bal > m2_bal);
        // Approximately 10000/12000 vs 2000/12000
        // m1 ~ 83.3%, m2 ~ 16.7%
        assert!(m1_bal > 4 * m2_bal); // at least 4x (accounting for rounding)
    }

    #[test]
    fn test_validator_reward_reduced_after_upgrade() {
        use crate::rewards::{distribute_block_reward, reward_per_block, MINING_UPGRADE_HEIGHT, VALIDATOR_REWARD_BPS, genesis_address_pub, NODE_INCENTIVE_POOL_INDEX};

        let mut state = WorldState::default();
        let pool_addr = genesis_address_pub(NODE_INCENTIVE_POOL_INDEX);
        state.balances.insert(pool_addr, 1_000_000_000_000);

        let validator = [1u8; 32];
        let validators = vec![(validator, 100u64)];

        // Before upgrade: validator gets 100% of reward
        let pool_before = state.get_balance(&pool_addr);
        distribute_block_reward(&mut state, &validators, 0);
        let pool_after = state.get_balance(&pool_addr);
        let validator_got_before = state.get_balance(&validator);
        let deducted_before = pool_before - pool_after;
        assert_eq!(validator_got_before, deducted_before); // 100%

        // Reset
        state.balances.insert(pool_addr, 1_000_000_000_000);
        state.balances.remove(&validator);

        // After upgrade: validator gets 65% of base reward
        distribute_block_reward(&mut state, &validators, MINING_UPGRADE_HEIGHT);
        let validator_got_after = state.get_balance(&validator);
        let base_reward = reward_per_block(MINING_UPGRADE_HEIGHT);
        let expected = base_reward * VALIDATOR_REWARD_BPS / 10000;
        assert_eq!(validator_got_after, expected);
    }

    #[test]
    fn test_update_miner_activity_deactivates() {
        use crate::rewards::update_miner_activity;

        let mut state = WorldState::default();
        let addr = [1u8; 32];
        state.miners.insert(addr, MinerInfo {
            address: addr,
            tier: MinerTier::Online,
            name: "miner".into(),
            registered_at: 0,
            last_heartbeat: 100,
            ip_prefix: vec![10, 0, 0],
            active: true,
            reputation_bps: REPUTATION_NEWCOMER_BPS,
            pending_rewards: 0,
            pending_epoch: 0,
            epoch_attendance: 0,
            consecutive_misses: 0,
        });

        // At height 100 + MINER_GRACE_BLOCKS - 1: still active
        update_miner_activity(&mut state, 100 + MINER_GRACE_BLOCKS - 1);
        assert!(state.miners.get(&addr).unwrap().active);

        // At height 100 + MINER_GRACE_BLOCKS + 1: deactivated
        update_miner_activity(&mut state, 100 + MINER_GRACE_BLOCKS + 1);
        assert!(!state.miners.get(&addr).unwrap().active);
    }

    // ======================== Heartbeat V2 Tests ========================

    /// First SETTLEMENT epoch boundary (the activation boundary only normalizes).
    fn first_v2_settlement_boundary() -> u64 {
        // V2_HEIGHT is the activation boundary (normalize only).
        // First real settlement is one epoch later.
        HEARTBEAT_V2_HEIGHT + MINER_EPOCH_LENGTH
    }

    /// Helper: create a WorldState at V2 height with pool funds and one active miner.
    /// Runs activation normalization, then returns state ready for the first settlement epoch.
    /// Returns (state, miner_address, first_settlement_boundary).
    fn setup_v2_miner() -> (WorldState, [u8; 32], u64) {
        use crate::rewards::{genesis_address_pub, NODE_INCENTIVE_POOL_INDEX, process_miner_epoch_boundary};

        let epoch_start = first_v2_settlement_boundary();

        let mut state = WorldState::default();
        let pool_addr = genesis_address_pub(NODE_INCENTIVE_POOL_INDEX);
        state.balances.insert(pool_addr, 1_000_000_000_000_000);

        let addr = [42u8; 32];
        state.miners.insert(addr, MinerInfo {
            address: addr,
            tier: MinerTier::Online,
            name: "v2-miner".into(),
            registered_at: HEARTBEAT_V2_HEIGHT,
            last_heartbeat: HEARTBEAT_V2_HEIGHT,
            ip_prefix: vec![10, 0, 0],
            active: true,
            reputation_bps: REPUTATION_VETERAN_BPS,
            pending_rewards: 0,
            pending_epoch: 0,
            epoch_attendance: 0xFFF,
            consecutive_misses: 0,
        });

        state.block_height = HEARTBEAT_V2_HEIGHT;

        // Run activation normalization (resets all V2 fields to zero)
        process_miner_epoch_boundary(&mut state, HEARTBEAT_V2_HEIGHT);

        // Simulate a miner that has built up attendance over prior epochs.
        // Without this, uptime=0% and no miner qualifies for rewards.
        state.miners.get_mut(&addr).unwrap().epoch_attendance = 0xFFF; // 12/12

        (state, addr, epoch_start)
    }

    #[test]
    fn test_v2_accumulate_fills_epoch_bucket() {
        use crate::rewards::accumulate_mining_reward;

        let (mut state, _addr, _) = setup_v2_miner();
        assert_eq!(state.epoch_reward_bucket, 0);

        let _ = accumulate_mining_reward(&mut state, HEARTBEAT_V2_HEIGHT);
        assert!(state.epoch_reward_bucket > 0);

        let bucket_after_1 = state.epoch_reward_bucket;
        let _ = accumulate_mining_reward(&mut state, HEARTBEAT_V2_HEIGHT + 1);
        assert_eq!(state.epoch_reward_bucket, bucket_after_1 * 2);
    }

    #[test]
    fn test_v2_epoch_boundary_pending_then_confirm() {
        use crate::rewards::{accumulate_mining_reward, process_miner_epoch_boundary};

        let (mut state, addr, epoch_start) = setup_v2_miner();

        // Accumulate for one full epoch leading up to the boundary
        let boundary = epoch_start + MINER_EPOCH_LENGTH;
        for h in epoch_start..boundary {
            accumulate_mining_reward(&mut state, h);
        }
        // Simulate heartbeat: set last_heartbeat within the settled epoch
        state.miners.get_mut(&addr).unwrap().last_heartbeat = epoch_start + 50;
        state.block_height = boundary;
        process_miner_epoch_boundary(&mut state, boundary);

        let miner = state.miners.get(&addr).unwrap();
        assert!(miner.pending_rewards > 0);
        assert_eq!(state.epoch_reward_bucket, 0);

        let balance_after_epoch1 = state.get_balance(&addr);

        // Next epoch: confirm previous pending
        let boundary2 = boundary + MINER_EPOCH_LENGTH;
        for h in boundary..boundary2 {
            accumulate_mining_reward(&mut state, h);
        }
        // Heartbeat in the second epoch
        state.miners.get_mut(&addr).unwrap().last_heartbeat = boundary + 50;
        state.block_height = boundary2;
        let events2 = process_miner_epoch_boundary(&mut state, boundary2);

        let balance_after_epoch2 = state.get_balance(&addr);
        assert!(balance_after_epoch2 > balance_after_epoch1);
        assert!(events2.iter().any(|e| matches!(e,
            claw_types::BlockEvent::RewardDistributed { reward_type, .. } if reward_type == "mining_reward_confirmed"
        )));
    }

    #[test]
    fn test_v2_missed_epoch_forfeits_pending() {
        use crate::rewards::{accumulate_mining_reward, process_miner_epoch_boundary, genesis_address_pub, NODE_INCENTIVE_POOL_INDEX};

        let (mut state, addr, epoch_start) = setup_v2_miner();
        let pool_addr = genesis_address_pub(NODE_INCENTIVE_POOL_INDEX);

        let boundary = epoch_start + MINER_EPOCH_LENGTH;
        for h in epoch_start..boundary {
            accumulate_mining_reward(&mut state, h);
        }
        state.miners.get_mut(&addr).unwrap().last_heartbeat = epoch_start + 50;
        process_miner_epoch_boundary(&mut state, boundary);
        let pending_amount = state.miners.get(&addr).unwrap().pending_rewards;
        assert!(pending_amount > 0);

        // Epoch 2: no check-in (last_heartbeat stays in epoch 1) → forfeit
        let boundary2 = boundary + MINER_EPOCH_LENGTH;
        for h in boundary..boundary2 {
            accumulate_mining_reward(&mut state, h);
        }
        let pool_after_accumulate = state.get_balance(&pool_addr);

        let events = process_miner_epoch_boundary(&mut state, boundary2);

        assert!(events.iter().any(|e| matches!(e,
            claw_types::BlockEvent::RewardDistributed { reward_type, .. } if reward_type == "mining_reward_forfeited"
        )));
        let pool_after_settle = state.get_balance(&pool_addr);
        assert!(pool_after_settle > pool_after_accumulate, "forfeited pending should return to pool");
    }

    #[test]
    fn test_v2_consecutive_misses_deactivate() {
        use crate::rewards::process_miner_epoch_boundary;

        let (mut state, addr, epoch_start) = setup_v2_miner();
        let mut height = epoch_start;

        for _ in 0..MINER_GRACE_EPOCHS {
            height += MINER_EPOCH_LENGTH;
            state.block_height = height;
            process_miner_epoch_boundary(&mut state, height);
        }

        let miner = state.miners.get(&addr).unwrap();
        assert!(!miner.active);
        assert_eq!(miner.consecutive_misses, MINER_GRACE_EPOCHS);
    }

    #[test]
    fn test_v2_reactivation_after_deactivation() {
        use crate::rewards::process_miner_epoch_boundary;

        let (mut state, addr, epoch_start) = setup_v2_miner();
        let mut height = epoch_start;

        for _ in 0..MINER_GRACE_EPOCHS {
            height += MINER_EPOCH_LENGTH;
            state.block_height = height;
            process_miner_epoch_boundary(&mut state, height);
        }
        assert!(!state.miners.get(&addr).unwrap().active);

        height += MINER_EPOCH_LENGTH;
        state.block_height = height;
        // Miner comes back online — heartbeat in the settled epoch
        let settled_epoch_start = height - MINER_EPOCH_LENGTH;
        state.miners.get_mut(&addr).unwrap().last_heartbeat = settled_epoch_start + 50;
        process_miner_epoch_boundary(&mut state, height);

        let miner = state.miners.get(&addr).unwrap();
        assert!(miner.active);
        assert_eq!(miner.consecutive_misses, 0);
    }

    #[test]
    fn test_v2_reputation_decay_on_miss() {
        use crate::rewards::process_miner_epoch_boundary;

        let (mut state, addr, epoch_start) = setup_v2_miner();
        let rep_before = state.miners.get(&addr).unwrap().reputation_bps;
        assert_eq!(rep_before, REPUTATION_VETERAN_BPS);

        let height = epoch_start + MINER_EPOCH_LENGTH;
        state.block_height = height;
        process_miner_epoch_boundary(&mut state, height);

        let rep_after = state.miners.get(&addr).unwrap().reputation_bps;
        assert_eq!(rep_after, 9900);
    }

    #[test]
    fn test_v2_uptime_below_threshold_no_reward() {
        use crate::rewards::{accumulate_mining_reward, process_miner_epoch_boundary};

        let (mut state, addr, epoch_start) = setup_v2_miner();
        state.miners.get_mut(&addr).unwrap().epoch_attendance = 0b0000_0000_0111; // 3 bits

        let boundary = epoch_start + MINER_EPOCH_LENGTH;
        for h in epoch_start..boundary {
            accumulate_mining_reward(&mut state, h);
        }
        state.miners.get_mut(&addr).unwrap().last_heartbeat = epoch_start + 50;
        process_miner_epoch_boundary(&mut state, boundary);

        let miner = state.miners.get(&addr).unwrap();
        assert_eq!(miner.pending_rewards, 0);
    }

    #[test]
    fn test_v2_heartbeat_handler_epoch_number_admission() {
        let (mut state, sk, addr) = setup_miner();
        // Move to V2 territory, new epoch relative to last heartbeat
        state.block_height = HEARTBEAT_V2_HEIGHT + MINER_EPOCH_LENGTH + 1;
        state.miners.get_mut(&addr).unwrap().last_heartbeat = HEARTBEAT_V2_HEIGHT;

        let payload = MinerHeartbeatPayload {
            latest_block_hash: [0u8; 32],
            latest_height: state.block_height,
        };
        let tx = make_tx(&sk, 2, TxType::MinerHeartbeat, &payload);
        state.apply_tx(&tx, 0).unwrap();

        assert!(state.epoch_checkins.contains_key(&addr));
    }

    #[test]
    fn test_state_root_includes_miners() {
        let (mut state, sk, _) = setup();
        let root_before = state.state_root();

        let payload = MinerRegisterPayload {
            tier: 1,
            ip_addr: vec![10, 0, 0, 1],
            name: "miner".into(),
        };
        state.apply_tx(&make_tx(&sk, 1, TxType::MinerRegister, &payload), 0).unwrap();

        let root_after = state.state_root();
        assert_ne!(root_before, root_after);
    }

    // === Contract block_timestamp fix (Task 0.1) ===

    /// Minimal WAT module that calls `block_timestamp()` and writes the
    /// 8-byte little-endian result into contract storage under the key b"ts".
    ///
    /// Only imports the two host functions it actually uses; other host functions
    /// don't need to be declared in a module that doesn't call them.
    /// Signatures match the actual host functions in `crates/vm/src/host.rs`:
    ///   block_timestamp() -> i64
    ///   storage_write(key_ptr: i32, key_len: i32, val_ptr: i32, val_len: i32)
    fn timestamp_contract_wasm() -> Vec<u8> {
        wat::parse_str(
            r#"
            (module
              (import "env" "block_timestamp" (func $block_timestamp (result i64)))
              (import "env" "storage_write"   (func $storage_write (param i32 i32 i32 i32)))
              (memory (export "memory") 1)
              ;; Memory layout:
              ;;   offset 0..2  = storage key "ts" (2 bytes, ASCII)
              ;;   offset 8..16 = i64 timestamp value (8 bytes, little-endian)
              (data (i32.const 0) "ts")
              (func (export "get_ts")
                (local $ts i64)
                (local.set $ts (call $block_timestamp))
                (i64.store (i32.const 8) (local.get $ts))
                (call $storage_write
                  (i32.const 0) (i32.const 2)
                  (i32.const 8) (i32.const 8))
              )
            )
            "#,
        )
        .expect("WAT compilation failed")
    }

    /// Verify that WorldState.block_timestamp defaults to 0 and can be set.
    ///
    /// TDD RED: compile-fails until `block_timestamp` field is added to WorldState.
    #[test]
    fn world_state_block_timestamp_field_exists() {
        let mut state = WorldState::default();
        assert_eq!(state.block_timestamp, 0, "default should be 0");
        state.block_timestamp = 9_999_999_999;
        assert_eq!(state.block_timestamp, 9_999_999_999);
    }

    /// Verify that `block_timestamp` is forwarded into the contract ExecutionContext
    /// so contracts calling `block_timestamp()` receive the actual block time,
    /// not a hardcoded 0.
    ///
    /// TDD RED: fails with stored_ts == 0 until handlers.rs is fixed.
    #[test]
    fn contract_call_execution_context_uses_block_timestamp() {
        let (sk, vk) = generate_keypair();
        let addr = vk.to_bytes();

        let mut state = WorldState::default();
        state.balances.insert(addr, 100 * GAS_FEE + 1_000_000_000);
        state.block_height = 42;
        // Set a known non-zero timestamp — this field must exist on WorldState.
        state.block_timestamp = 1_700_000_042;

        // Deploy the timestamp-reading contract (no constructor).
        let code = timestamp_contract_wasm();
        let deploy_payload = ContractDeployPayload {
            code,
            init_method: String::new(),
            init_args: vec![],
        };
        let deploy_tx = make_tx(&sk, 1, TxType::ContractDeploy, &deploy_payload);
        state.apply_tx(&deploy_tx, 0).expect("deploy failed");

        // Derive the contract address the same way the handler does.
        // nonce was 0 before the deploy transaction consumed it.
        let contract_addr = claw_vm::VmEngine::derive_contract_address(&addr, 0);
        assert!(state.contracts.contains_key(&contract_addr), "contract not registered");

        // Call get_ts — should store block_timestamp into contract storage under key "ts".
        let call_payload = ContractCallPayload {
            contract: contract_addr,
            method: "get_ts".into(),
            args: vec![],
            value: 0,
        };
        let call_tx = make_tx(&sk, 2, TxType::ContractCall, &call_payload);
        state.apply_tx(&call_tx, 0).expect("contract call failed");

        // Read the stored value: must equal the timestamp we set, NOT zero.
        let stored = state
            .contract_storage
            .get(&(contract_addr, b"ts".to_vec()))
            .expect("storage key 'ts' not written by contract");

        assert_eq!(stored.len(), 8, "expected 8-byte i64 in storage");
        let stored_ts = i64::from_le_bytes(stored[..8].try_into().unwrap()) as u64;
        assert_eq!(
            stored_ts,
            1_700_000_042,
            "contract received block_timestamp={stored_ts} but expected 1_700_000_042; \
             handlers.rs likely still has `block_timestamp: 0` hardcoded"
        );
    }

    // === Contract Upgrade Mechanism (Timelock) ===

    /// Minimal WAT module used for upgrade tests.
    /// Exports a `noop` method so the contract is valid and callable.
    fn minimal_contract_wasm() -> Vec<u8> {
        wat::parse_str(
            r#"
            (module
              (memory (export "memory") 1)
              (func (export "noop"))
            )
            "#,
        )
        .expect("WAT compilation failed")
    }

    /// A second minimal WAT module with a different body (different code_hash).
    fn minimal_contract_v2_wasm() -> Vec<u8> {
        wat::parse_str(
            r#"
            (module
              (memory (export "memory") 1)
              (func (export "noop")
                (nop)
                (nop)
              )
            )
            "#,
        )
        .expect("WAT compilation failed")
    }

    /// Helper: deploy a minimal contract and return its address.
    fn deploy_minimal_contract(
        state: &mut WorldState,
        sk: &SigningKey,
        nonce: u64,
    ) -> [u8; 32] {
        let addr = sk.verifying_key().to_bytes();
        let code = minimal_contract_wasm();
        let payload = ContractDeployPayload {
            code,
            init_method: String::new(),
            init_args: vec![],
        };
        let tx = make_tx(sk, nonce, TxType::ContractDeploy, &payload);
        state.apply_tx(&tx, 0).expect("deploy should succeed");
        let pre_deploy_nonce = nonce - 1;
        claw_vm::VmEngine::derive_contract_address(&addr, pre_deploy_nonce)
    }

    /// Test 1: Announce upgrade stores PendingUpgrade on the contract instance.
    #[test]
    fn contract_upgrade_announce_stores_pending_upgrade() {
        let (mut state, sk, _addr) = setup();
        state.block_height = 100;

        let contract_addr = deploy_minimal_contract(&mut state, &sk, 1);

        let new_code = minimal_contract_v2_wasm();
        let new_code_hash = *blake3::hash(&new_code).as_bytes();

        let announce_payload = ContractUpgradeAnnouncePayload {
            contract: contract_addr,
            new_code_hash,
        };
        let announce_tx = make_tx(&sk, 2, TxType::ContractUpgradeAnnounce, &announce_payload);
        state.apply_tx(&announce_tx, 0).expect("announce should succeed");

        let instance = state.contracts.get(&contract_addr).expect("instance must exist");
        let pending = instance.pending_upgrade.as_ref().expect("pending_upgrade must be set");
        assert_eq!(pending.new_code_hash, new_code_hash);
        assert_eq!(pending.announced_at, 100);
        assert_eq!(
            pending.ready_at,
            100 + crate::UPGRADE_DELAY_BLOCKS,
            "ready_at must be announced_at + UPGRADE_DELAY_BLOCKS"
        );
    }

    /// Test 2: Execute upgrade before ready_at is rejected.
    #[test]
    fn contract_upgrade_execute_before_ready_rejected() {
        let (mut state, sk, _addr) = setup();
        state.block_height = 100;

        let contract_addr = deploy_minimal_contract(&mut state, &sk, 1);

        let new_code = minimal_contract_v2_wasm();
        let new_code_hash = *blake3::hash(&new_code).as_bytes();

        let announce_payload = ContractUpgradeAnnouncePayload {
            contract: contract_addr,
            new_code_hash,
        };
        let announce_tx = make_tx(&sk, 2, TxType::ContractUpgradeAnnounce, &announce_payload);
        state.apply_tx(&announce_tx, 0).expect("announce should succeed");

        // Try to execute at block 100 (same block as announce, before ready_at)
        state.block_height = 100;
        let execute_payload = ContractUpgradeExecutePayload {
            contract: contract_addr,
            new_code: new_code.clone(),
            migrate_method: None,
            migrate_args: vec![],
        };
        let execute_tx = make_tx(&sk, 3, TxType::ContractUpgradeExecute, &execute_payload);
        let result = state.apply_tx(&execute_tx, 0);
        assert!(
            result.is_err(),
            "executing upgrade before ready_at must fail"
        );
        assert!(
            matches!(result.unwrap_err(), StateError::UpgradeNotReady { .. }),
            "error must be UpgradeNotReady"
        );
    }

    /// Test 3: Execute upgrade after ready_at replaces code and updates instance.
    #[test]
    fn contract_upgrade_execute_after_ready_succeeds() {
        let (mut state, sk, _addr) = setup();
        state.block_height = 100;

        let contract_addr = deploy_minimal_contract(&mut state, &sk, 1);
        let original_code_hash = state.contracts[&contract_addr].code_hash;

        let new_code = minimal_contract_v2_wasm();
        let new_code_hash = *blake3::hash(&new_code).as_bytes();

        // Announce at block 100
        let announce_payload = ContractUpgradeAnnouncePayload {
            contract: contract_addr,
            new_code_hash,
        };
        let announce_tx = make_tx(&sk, 2, TxType::ContractUpgradeAnnounce, &announce_payload);
        state.apply_tx(&announce_tx, 0).expect("announce should succeed");

        // Advance block height past ready_at
        state.block_height = 100 + crate::UPGRADE_DELAY_BLOCKS;

        let execute_payload = ContractUpgradeExecutePayload {
            contract: contract_addr,
            new_code: new_code.clone(),
            migrate_method: None,
            migrate_args: vec![],
        };
        let execute_tx = make_tx(&sk, 3, TxType::ContractUpgradeExecute, &execute_payload);
        state.apply_tx(&execute_tx, 0).expect("execute should succeed after delay");

        let instance = state.contracts.get(&contract_addr).expect("instance must exist");
        assert_eq!(instance.code_hash, new_code_hash, "code_hash must be updated");
        assert_eq!(
            instance.previous_code_hash,
            Some(original_code_hash),
            "previous_code_hash must be set to the old hash"
        );
        assert_eq!(
            instance.upgrade_height,
            Some(100 + crate::UPGRADE_DELAY_BLOCKS),
            "upgrade_height must be set to current block"
        );
        assert!(
            instance.pending_upgrade.is_none(),
            "pending_upgrade must be cleared after execution"
        );

        // Verify the actual bytecode in contract_code was replaced
        let stored_code = state.contract_code.get(&contract_addr).expect("code must exist");
        assert_eq!(*blake3::hash(stored_code).as_bytes(), new_code_hash);
    }

    /// Test 4: Execute with wrong code hash (code does not match announced hash) is rejected.
    #[test]
    fn contract_upgrade_execute_wrong_code_hash_rejected() {
        let (mut state, sk, _addr) = setup();
        state.block_height = 100;

        let contract_addr = deploy_minimal_contract(&mut state, &sk, 1);

        let new_code = minimal_contract_v2_wasm();
        let new_code_hash = *blake3::hash(&new_code).as_bytes();

        // Announce with hash of v2
        let announce_payload = ContractUpgradeAnnouncePayload {
            contract: contract_addr,
            new_code_hash,
        };
        let announce_tx = make_tx(&sk, 2, TxType::ContractUpgradeAnnounce, &announce_payload);
        state.apply_tx(&announce_tx, 0).expect("announce should succeed");

        state.block_height = 100 + crate::UPGRADE_DELAY_BLOCKS;

        // Submit the original (v1) code, which has a different hash
        let wrong_code = minimal_contract_wasm();
        let execute_payload = ContractUpgradeExecutePayload {
            contract: contract_addr,
            new_code: wrong_code,
            migrate_method: None,
            migrate_args: vec![],
        };
        let execute_tx = make_tx(&sk, 3, TxType::ContractUpgradeExecute, &execute_payload);
        let result = state.apply_tx(&execute_tx, 0);
        assert!(result.is_err(), "wrong code hash must be rejected");
        assert!(
            matches!(result.unwrap_err(), StateError::UpgradeCodeHashMismatch),
            "error must be UpgradeCodeHashMismatch"
        );
    }

    /// Test 5: Non-admin announce is rejected.
    #[test]
    fn contract_upgrade_announce_non_admin_rejected() {
        let (mut state, sk, _addr) = setup();
        state.block_height = 100;

        let contract_addr = deploy_minimal_contract(&mut state, &sk, 1);

        // Create a different keypair (the attacker)
        let (attacker_sk, attacker_vk) = generate_keypair();
        let attacker_addr = attacker_vk.to_bytes();
        state
            .balances
            .insert(attacker_addr, 100 * GAS_FEE + 1_000_000_000);

        let new_code = minimal_contract_v2_wasm();
        let new_code_hash = *blake3::hash(&new_code).as_bytes();

        let announce_payload = ContractUpgradeAnnouncePayload {
            contract: contract_addr,
            new_code_hash,
        };
        // Attacker tries to announce upgrade for a contract they don't own
        let announce_tx = make_tx(&attacker_sk, 1, TxType::ContractUpgradeAnnounce, &announce_payload);
        let result = state.apply_tx(&announce_tx, 0);
        assert!(result.is_err(), "non-admin announce must be rejected");
        assert!(
            matches!(result.unwrap_err(), StateError::Unauthorized),
            "error must be Unauthorized"
        );
    }

    /// Test 6: Non-admin execute is rejected.
    #[test]
    fn contract_upgrade_execute_non_admin_rejected() {
        let (mut state, sk, _addr) = setup();
        state.block_height = 100;

        let contract_addr = deploy_minimal_contract(&mut state, &sk, 1);

        let new_code = minimal_contract_v2_wasm();
        let new_code_hash = *blake3::hash(&new_code).as_bytes();

        // Admin announces
        let announce_payload = ContractUpgradeAnnouncePayload {
            contract: contract_addr,
            new_code_hash,
        };
        let announce_tx = make_tx(&sk, 2, TxType::ContractUpgradeAnnounce, &announce_payload);
        state.apply_tx(&announce_tx, 0).expect("announce should succeed");

        state.block_height = 100 + crate::UPGRADE_DELAY_BLOCKS;

        // Create an attacker
        let (attacker_sk, attacker_vk) = generate_keypair();
        let attacker_addr = attacker_vk.to_bytes();
        state
            .balances
            .insert(attacker_addr, 100 * GAS_FEE + 1_000_000_000);

        let execute_payload = ContractUpgradeExecutePayload {
            contract: contract_addr,
            new_code: new_code.clone(),
            migrate_method: None,
            migrate_args: vec![],
        };
        let execute_tx = make_tx(&attacker_sk, 1, TxType::ContractUpgradeExecute, &execute_payload);
        let result = state.apply_tx(&execute_tx, 0);
        assert!(result.is_err(), "non-admin execute must be rejected");
        assert!(
            matches!(result.unwrap_err(), StateError::Unauthorized),
            "error must be Unauthorized"
        );
    }

    /// Test 7: Execute with migrate_method=Some runs migration against existing storage.
    #[test]
    fn contract_upgrade_execute_with_migration_runs_migrate() {
        let (mut state, sk, _addr) = setup();
        state.block_height = 100;

        // Deploy the timestamp contract as a stand-in for a "real" contract
        // (it has a `get_ts` method we can call post-upgrade)
        let (sk2, vk2) = generate_keypair();
        let addr2 = vk2.to_bytes();
        state.balances.insert(addr2, 100 * GAS_FEE + 1_000_000_000);

        let contract_addr = deploy_minimal_contract(&mut state, &sk, 1);

        let new_code = minimal_contract_v2_wasm();
        let new_code_hash = *blake3::hash(&new_code).as_bytes();

        // Announce
        let announce_payload = ContractUpgradeAnnouncePayload {
            contract: contract_addr,
            new_code_hash,
        };
        let announce_tx = make_tx(&sk, 2, TxType::ContractUpgradeAnnounce, &announce_payload);
        state.apply_tx(&announce_tx, 0).expect("announce should succeed");

        state.block_height = 100 + crate::UPGRADE_DELAY_BLOCKS;

        // Execute with migrate_method=Some("noop") — noop exists in v2 wasm
        let execute_payload = ContractUpgradeExecutePayload {
            contract: contract_addr,
            new_code: new_code.clone(),
            migrate_method: Some("noop".to_string()),
            migrate_args: vec![],
        };
        let execute_tx = make_tx(&sk, 3, TxType::ContractUpgradeExecute, &execute_payload);
        // Should succeed — migration runs "noop" which does nothing
        state.apply_tx(&execute_tx, 0).expect("execute with migration should succeed");

        // Contract instance should be updated
        let instance = state.contracts.get(&contract_addr).expect("instance must exist");
        assert_eq!(instance.code_hash, new_code_hash);
        assert!(instance.pending_upgrade.is_none());
    }

    /// Test 8: previous_code_hash is updated correctly after upgrade.
    /// (Implicitly covered by test 3, but we make it explicit.)
    #[test]
    fn contract_upgrade_previous_code_hash_updated() {
        let (mut state, sk, _addr) = setup();
        state.block_height = 50;

        let contract_addr = deploy_minimal_contract(&mut state, &sk, 1);
        let v1_hash = state.contracts[&contract_addr].code_hash;

        let v2_code = minimal_contract_v2_wasm();
        let v2_hash = *blake3::hash(&v2_code).as_bytes();

        let announce_payload = ContractUpgradeAnnouncePayload {
            contract: contract_addr,
            new_code_hash: v2_hash,
        };
        let announce_tx = make_tx(&sk, 2, TxType::ContractUpgradeAnnounce, &announce_payload);
        state.apply_tx(&announce_tx, 0).expect("announce should succeed");

        state.block_height = 50 + crate::UPGRADE_DELAY_BLOCKS;

        let execute_payload = ContractUpgradeExecutePayload {
            contract: contract_addr,
            new_code: v2_code,
            migrate_method: None,
            migrate_args: vec![],
        };
        let execute_tx = make_tx(&sk, 3, TxType::ContractUpgradeExecute, &execute_payload);
        state.apply_tx(&execute_tx, 0).expect("execute should succeed");

        let instance = state.contracts.get(&contract_addr).expect("instance must exist");
        assert_eq!(
            instance.previous_code_hash,
            Some(v1_hash),
            "previous_code_hash should equal the v1 hash"
        );
        assert_eq!(instance.code_hash, v2_hash);
    }

    /// Test 9: Re-announcing replaces the pending upgrade (cancel via new announce).
    #[test]
    fn contract_upgrade_re_announce_replaces_pending() {
        let (mut state, sk, _addr) = setup();
        state.block_height = 100;

        let contract_addr = deploy_minimal_contract(&mut state, &sk, 1);

        let v2_code = minimal_contract_v2_wasm();
        let v2_hash = *blake3::hash(&v2_code).as_bytes();

        // First announce with v2 hash
        let announce1 = ContractUpgradeAnnouncePayload {
            contract: contract_addr,
            new_code_hash: v2_hash,
        };
        let tx1 = make_tx(&sk, 2, TxType::ContractUpgradeAnnounce, &announce1);
        state.apply_tx(&tx1, 0).expect("first announce should succeed");

        // Second announce with a different (v1 code hash as a stand-in for "v3")
        let v1_code = minimal_contract_wasm();
        let v3_hash = *blake3::hash(&v1_code).as_bytes(); // reuse v1 bytes as "v3"

        state.block_height = 200; // advance height for the second announce
        let announce2 = ContractUpgradeAnnouncePayload {
            contract: contract_addr,
            new_code_hash: v3_hash,
        };
        let tx2 = make_tx(&sk, 3, TxType::ContractUpgradeAnnounce, &announce2);
        state.apply_tx(&tx2, 0).expect("second announce should succeed");

        let instance = state.contracts.get(&contract_addr).expect("instance must exist");
        let pending = instance.pending_upgrade.as_ref().expect("pending must exist");
        assert_eq!(
            pending.new_code_hash, v3_hash,
            "pending upgrade must be updated to the latest announce"
        );
        assert_eq!(
            pending.announced_at, 200,
            "announced_at must reflect the new announcement block"
        );
    }

    /// Test 10: Announce on non-existent contract fails.
    #[test]
    fn contract_upgrade_announce_nonexistent_contract_fails() {
        let (mut state, sk, _addr) = setup();
        state.block_height = 100;

        let fake_contract = [0x42u8; 32];
        let new_code_hash = [0x01u8; 32];

        let announce_payload = ContractUpgradeAnnouncePayload {
            contract: fake_contract,
            new_code_hash,
        };
        let tx = make_tx(&sk, 1, TxType::ContractUpgradeAnnounce, &announce_payload);
        let result = state.apply_tx(&tx, 0);
        assert!(result.is_err(), "announce on nonexistent contract must fail");
        assert!(
            matches!(result.unwrap_err(), StateError::ContractNotFound(_)),
            "error must be ContractNotFound"
        );
    }

    /// Test 11: Execute without a pending upgrade fails.
    #[test]
    fn contract_upgrade_execute_without_announce_fails() {
        let (mut state, sk, _addr) = setup();
        state.block_height = 1000;

        let contract_addr = deploy_minimal_contract(&mut state, &sk, 1);

        let new_code = minimal_contract_v2_wasm();
        let execute_payload = ContractUpgradeExecutePayload {
            contract: contract_addr,
            new_code,
            migrate_method: None,
            migrate_args: vec![],
        };
        let execute_tx = make_tx(&sk, 2, TxType::ContractUpgradeExecute, &execute_payload);
        let result = state.apply_tx(&execute_tx, 0);
        assert!(result.is_err(), "execute without pending upgrade must fail");
        assert!(
            matches!(result.unwrap_err(), StateError::NoPendingUpgrade),
            "error must be NoPendingUpgrade"
        );
    }

    /// Test 12: admin field defaults to creator on deploy.
    #[test]
    fn contract_instance_admin_defaults_to_creator() {
        let (mut state, sk, addr) = setup();
        state.block_height = 1;

        let contract_addr = deploy_minimal_contract(&mut state, &sk, 1);

        let instance = state.contracts.get(&contract_addr).expect("instance must exist");
        assert_eq!(
            instance.admin, addr,
            "admin must default to creator on deploy"
        );
    }

    /// Verify that the contract deploy constructor also receives the correct
    /// block_timestamp (not 0) when an init_method is provided.
    #[test]
    fn contract_deploy_constructor_uses_block_timestamp() {
        let (sk, vk) = generate_keypair();
        let addr = vk.to_bytes();

        let mut state = WorldState::default();
        state.balances.insert(addr, 100 * GAS_FEE + 1_000_000_000);
        state.block_height = 10;
        state.block_timestamp = 1_600_000_010;

        // Deploy with init_method="get_ts" so the constructor runs our timestamp-writing code.
        let code = timestamp_contract_wasm();
        let deploy_payload = ContractDeployPayload {
            code,
            init_method: "get_ts".into(),
            init_args: vec![],
        };
        let deploy_tx = make_tx(&sk, 1, TxType::ContractDeploy, &deploy_payload);
        state.apply_tx(&deploy_tx, 0).expect("deploy with constructor failed");

        let contract_addr = claw_vm::VmEngine::derive_contract_address(&addr, 0);

        // The constructor stored block_timestamp under key "ts".
        let stored = state
            .contract_storage
            .get(&(contract_addr, b"ts".to_vec()))
            .expect("constructor did not write storage key 'ts'");

        let stored_ts = i64::from_le_bytes(stored[..8].try_into().unwrap()) as u64;
        assert_eq!(
            stored_ts,
            1_600_000_010,
            "constructor received block_timestamp={stored_ts} but expected 1_600_000_010; \
             ContractDeploy handler likely still has `block_timestamp: 0` hardcoded"
        );
    }
}
