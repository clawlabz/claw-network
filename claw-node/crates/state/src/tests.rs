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
        assert!(state.apply_tx(&tx).is_ok());
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
        state.apply_tx(&tx1).unwrap();

        let tx2 = make_tx(&sk, 2, TxType::AgentRegister, &payload);
        assert_eq!(
            state.apply_tx(&tx2),
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
        assert!(matches!(state.apply_tx(&tx), Err(StateError::NameTooLong { .. })));
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
        state.apply_tx(&tx).unwrap();

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
            state.apply_tx(&tx),
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
        assert_eq!(state.apply_tx(&tx), Err(StateError::ZeroAmount));
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
            state.apply_tx(&tx),
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

        let payload = AgentRegisterPayload {
            name: "agent".into(),
            metadata: BTreeMap::new(),
        };
        let tx = make_tx(&sk, 1, TxType::AgentRegister, &payload);
        assert!(matches!(
            state.apply_tx(&tx),
            Err(StateError::InsufficientBalance { .. })
        ));
    }

    #[test]
    fn gas_burned_on_success() {
        let (mut state, sk, addr) = setup();
        let initial = state.get_balance(&addr);
        let payload = AgentRegisterPayload {
            name: "agent".into(),
            metadata: BTreeMap::new(),
        };
        let tx = make_tx(&sk, 1, TxType::AgentRegister, &payload);
        state.apply_tx(&tx).unwrap();
        assert_eq!(state.get_balance(&addr), initial - GAS_FEE);
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
        state.apply_tx(&tx1).unwrap();

        let balance_after_first = state.get_balance(&addr);

        // Try duplicate register — should fail, gas refunded
        let tx2 = make_tx(&sk, 2, TxType::AgentRegister, &payload);
        assert!(state.apply_tx(&tx2).is_err());
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
        state.apply_tx(&make_tx(&sk, 1, TxType::AgentRegister, &reg)).unwrap();

        let payload = TokenCreatePayload {
            name: "TestCoin".into(),
            symbol: "TST".into(),
            decimals: 6,
            total_supply: 1_000_000,
        };
        let tx = make_tx(&sk, 2, TxType::TokenCreate, &payload);
        state.apply_tx(&tx).unwrap();

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
        assert_eq!(state.apply_tx(&tx), Err(StateError::AgentNotRegistered));
    }

    // === Token Mint Transfer ===

    #[test]
    fn token_mint_transfer_success() {
        let (mut state, sk, addr) = setup();
        let (_, vk2) = generate_keypair();
        let addr2 = vk2.to_bytes();

        // Register + create token
        let reg = AgentRegisterPayload { name: "a".into(), metadata: BTreeMap::new() };
        state.apply_tx(&make_tx(&sk, 1, TxType::AgentRegister, &reg)).unwrap();
        let create = TokenCreatePayload {
            name: "Coin".into(), symbol: "C".into(), decimals: 0, total_supply: 100,
        };
        state.apply_tx(&make_tx(&sk, 2, TxType::TokenCreate, &create)).unwrap();
        let token_id = *state.tokens.keys().next().unwrap();

        // Transfer custom token
        let payload = TokenMintTransferPayload { token_id, to: addr2, amount: 30 };
        state.apply_tx(&make_tx(&sk, 3, TxType::TokenMintTransfer, &payload)).unwrap();

        assert_eq!(state.get_token_balance(&addr, &token_id), 70);
        assert_eq!(state.get_token_balance(&addr2, &token_id), 30);
    }

    #[test]
    fn token_mint_transfer_native_id_fails() {
        let (mut state, sk, _) = setup();
        let reg = AgentRegisterPayload { name: "a".into(), metadata: BTreeMap::new() };
        state.apply_tx(&make_tx(&sk, 1, TxType::AgentRegister, &reg)).unwrap();

        let payload = TokenMintTransferPayload {
            token_id: NATIVE_TOKEN_ID,
            to: [2u8; 32],
            amount: 10,
        };
        let tx = make_tx(&sk, 2, TxType::TokenMintTransfer, &payload);
        assert_eq!(state.apply_tx(&tx), Err(StateError::NativeTokenIdForCustom));
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
        state.apply_tx(&make_tx(&sk1, 1, TxType::AgentRegister, &reg1)).unwrap();
        let reg2 = AgentRegisterPayload { name: "a2".into(), metadata: BTreeMap::new() };
        state.apply_tx(&make_tx(&sk2, 1, TxType::AgentRegister, &reg2)).unwrap();

        let payload = ReputationAttestPayload {
            to: addr2,
            category: "game".into(),
            score: 80,
            platform: "arena".into(),
            memo: "good player".into(),
        };
        state.apply_tx(&make_tx(&sk1, 2, TxType::ReputationAttest, &payload)).unwrap();

        assert_eq!(state.reputation.len(), 1);
        assert_eq!(state.reputation[0].score, 80);
    }

    #[test]
    fn reputation_self_attest_fails() {
        let (mut state, sk, addr) = setup();
        let reg = AgentRegisterPayload { name: "a".into(), metadata: BTreeMap::new() };
        state.apply_tx(&make_tx(&sk, 1, TxType::AgentRegister, &reg)).unwrap();

        let payload = ReputationAttestPayload {
            to: addr,
            category: "game".into(),
            score: 100,
            platform: "arena".into(),
            memo: "".into(),
        };
        let tx = make_tx(&sk, 2, TxType::ReputationAttest, &payload);
        assert_eq!(state.apply_tx(&tx), Err(StateError::SelfAttestation));
    }

    #[test]
    fn reputation_score_out_of_range() {
        let (mut state, sk1, _) = setup();
        let (sk2, vk2) = generate_keypair();
        let addr2 = vk2.to_bytes();
        state.balances.insert(addr2, 100 * GAS_FEE);

        let reg1 = AgentRegisterPayload { name: "a1".into(), metadata: BTreeMap::new() };
        state.apply_tx(&make_tx(&sk1, 1, TxType::AgentRegister, &reg1)).unwrap();
        let reg2 = AgentRegisterPayload { name: "a2".into(), metadata: BTreeMap::new() };
        state.apply_tx(&make_tx(&sk2, 1, TxType::AgentRegister, &reg2)).unwrap();

        let payload = ReputationAttestPayload {
            to: addr2, category: "x".into(), score: 101, platform: "p".into(), memo: "".into(),
        };
        assert_eq!(
            state.apply_tx(&make_tx(&sk1, 2, TxType::ReputationAttest, &payload)),
            Err(StateError::ScoreOutOfRange(101))
        );
    }

    // === Service Register ===

    #[test]
    fn service_register_success() {
        let (mut state, sk, addr) = setup();
        let reg = AgentRegisterPayload { name: "a".into(), metadata: BTreeMap::new() };
        state.apply_tx(&make_tx(&sk, 1, TxType::AgentRegister, &reg)).unwrap();

        let payload = ServiceRegisterPayload {
            service_type: "translation".into(),
            description: "EN-CN".into(),
            price_token: NATIVE_TOKEN_ID,
            price_amount: 10_000_000,
            endpoint: "https://example.com/translate".into(),
            active: true,
        };
        state.apply_tx(&make_tx(&sk, 2, TxType::ServiceRegister, &payload)).unwrap();

        let svc = state.services.get(&(addr, "translation".to_string())).unwrap();
        assert_eq!(svc.endpoint, "https://example.com/translate");
        assert!(svc.active);
    }

    #[test]
    fn service_register_upsert() {
        let (mut state, sk, addr) = setup();
        let reg = AgentRegisterPayload { name: "a".into(), metadata: BTreeMap::new() };
        state.apply_tx(&make_tx(&sk, 1, TxType::AgentRegister, &reg)).unwrap();

        let payload1 = ServiceRegisterPayload {
            service_type: "review".into(),
            description: "v1".into(),
            price_token: NATIVE_TOKEN_ID,
            price_amount: 100,
            endpoint: "https://v1.com".into(),
            active: true,
        };
        state.apply_tx(&make_tx(&sk, 2, TxType::ServiceRegister, &payload1)).unwrap();

        // Update same service type
        let payload2 = ServiceRegisterPayload {
            service_type: "review".into(),
            description: "v2".into(),
            price_token: NATIVE_TOKEN_ID,
            price_amount: 200,
            endpoint: "https://v2.com".into(),
            active: false,
        };
        state.apply_tx(&make_tx(&sk, 3, TxType::ServiceRegister, &payload2)).unwrap();

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
        state.apply_tx(&make_tx(&sk, 1, TxType::AgentRegister, &payload)).unwrap();

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
        state.apply_tx(&make_tx(&sk, 1, TxType::AgentRegister, &payload)).unwrap();

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
        assert_eq!(state.apply_tx(&tx), Err(StateError::InvalidSignature));
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
        state.apply_tx(&make_tx(&sk1, 1, TxType::AgentRegister, &reg1)).unwrap();
        let reg2 = AgentRegisterPayload { name: "agent2".into(), metadata: BTreeMap::new() };
        state.apply_tx(&make_tx(&sk2, 1, TxType::AgentRegister, &reg2)).unwrap();

        // Stake 50k CLAW for addr1 (Platform Agent threshold)
        let stake = claw_types::transaction::StakeDepositPayload { amount: 50_000_000_000_000, validator: [0u8; 32], commission_bps: 10000 };
        state.apply_tx(&make_tx(&sk1, 2, TxType::StakeDeposit, &stake)).unwrap();

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
        state.apply_tx(&tx).unwrap();

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
            state.apply_tx(&tx),
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
        state.apply_tx(&make_tx(&sk1, 3, TxType::PlatformActivityReport, &payload)).unwrap();

        // Second report in same epoch fails
        let tx2 = make_tx(&sk1, 4, TxType::PlatformActivityReport, &payload);
        assert_eq!(
            state.apply_tx(&tx2),
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
            state.apply_tx(&tx),
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
            state.apply_tx(&tx),
            Err(StateError::AgentNotRegistered)
        );
    }

    // === Activity Stats Tracking ===

    #[test]
    fn activity_stats_updated_on_tx() {
        let (mut state, sk, addr) = setup();

        // Register agent (should increment tx_count)
        let reg = AgentRegisterPayload { name: "agent".into(), metadata: BTreeMap::new() };
        state.apply_tx(&make_tx(&sk, 1, TxType::AgentRegister, &reg)).unwrap();

        let stats = state.activity_stats.get(&addr).unwrap();
        assert_eq!(stats.tx_count, 1);
        assert!(stats.gas_consumed > 0);

        // Token create (should also increment tokens_created)
        let create = TokenCreatePayload {
            name: "Test".into(),
            symbol: "T".into(),
            decimals: 0,
            total_supply: 100,
        };
        state.apply_tx(&make_tx(&sk, 2, TxType::TokenCreate, &create)).unwrap();

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
        state.apply_tx(&tx).unwrap();

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
        state.apply_tx(&make_tx(&sk, 1, TxType::MinerRegister, &payload)).unwrap();

        let tx2 = make_tx(&sk, 2, TxType::MinerRegister, &payload);
        assert_eq!(state.apply_tx(&tx2), Err(StateError::MinerAlreadyRegistered));
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
        assert_eq!(state.apply_tx(&tx), Err(StateError::InvalidMinerTier(5)));
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
            state.apply_tx(&tx).unwrap();
        }

        // 4th miner on same subnet should fail
        let payload = MinerRegisterPayload {
            tier: 1,
            ip_addr: vec![10, 0, 0, 99],
            name: "miner-overflow".into(),
        };
        let tx = make_tx(&keys[3].0, 1, TxType::MinerRegister, &payload);
        assert_eq!(
            state.apply_tx(&tx),
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
        assert_eq!(state.apply_tx(&tx), Err(StateError::InvalidIpLength(2)));
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
        state.apply_tx(&make_tx(&sk, 1, TxType::MinerRegister, &payload)).unwrap();
        (state, sk, addr)
    }

    #[test]
    fn test_miner_heartbeat_success() {
        let (mut state, sk, addr) = setup_miner();
        // Advance block_height past the heartbeat interval
        state.block_height = MINER_HEARTBEAT_INTERVAL + 1;

        let payload = MinerHeartbeatPayload {
            latest_block_hash: [0u8; 32],
            latest_height: state.block_height,
        };
        let tx = make_tx(&sk, 2, TxType::MinerHeartbeat, &payload);
        state.apply_tx(&tx).unwrap();

        let miner = state.miners.get(&addr).unwrap();
        assert_eq!(miner.last_heartbeat, MINER_HEARTBEAT_INTERVAL + 1);
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
        assert_eq!(state.apply_tx(&tx), Err(StateError::MinerNotRegistered));
    }

    #[test]
    fn test_miner_heartbeat_too_early() {
        let (mut state, sk, _) = setup_miner();
        // Don't advance block_height past interval (miner registered at height 0)
        state.block_height = 500; // < MINER_HEARTBEAT_INTERVAL

        let payload = MinerHeartbeatPayload {
            latest_block_hash: [0u8; 32],
            latest_height: 500,
        };
        let tx = make_tx(&sk, 2, TxType::MinerHeartbeat, &payload);
        assert!(matches!(
            state.apply_tx(&tx),
            Err(StateError::HeartbeatTooEarly { .. })
        ));
    }

    #[test]
    fn test_miner_heartbeat_gas_free() {
        let (mut state, sk, addr) = setup_miner();
        let balance_before = state.get_balance(&addr);
        state.block_height = MINER_HEARTBEAT_INTERVAL + 1;

        let payload = MinerHeartbeatPayload {
            latest_block_hash: [0u8; 32],
            latest_height: state.block_height,
        };
        let tx = make_tx(&sk, 2, TxType::MinerHeartbeat, &payload);
        let fee = state.apply_tx(&tx).unwrap();

        // Gas should be 0
        assert_eq!(fee, 0);
        // Balance should be unchanged (no gas deducted)
        assert_eq!(state.get_balance(&addr), balance_before);
    }

    #[test]
    fn test_miner_heartbeat_updates_reputation() {
        let (mut state, sk, addr) = setup_miner();

        // At newcomer stage (< 7 days)
        state.block_height = MINER_HEARTBEAT_INTERVAL + 1;
        let hb = MinerHeartbeatPayload {
            latest_block_hash: [0u8; 32],
            latest_height: state.block_height,
        };
        state.apply_tx(&make_tx(&sk, 2, TxType::MinerHeartbeat, &hb)).unwrap();
        assert_eq!(state.miners.get(&addr).unwrap().reputation_bps, REPUTATION_NEWCOMER_BPS);

        // Advance to established stage (>= 7 days from registration)
        state.block_height = BLOCKS_7_DAYS + MINER_HEARTBEAT_INTERVAL + 1;
        let hb2 = MinerHeartbeatPayload {
            latest_block_hash: [0u8; 32],
            latest_height: state.block_height,
        };
        state.apply_tx(&make_tx(&sk, 3, TxType::MinerHeartbeat, &hb2)).unwrap();
        assert_eq!(state.miners.get(&addr).unwrap().reputation_bps, REPUTATION_ESTABLISHED_BPS);

        // Advance to veteran stage (>= 30 days from registration)
        state.block_height = BLOCKS_30_DAYS + MINER_HEARTBEAT_INTERVAL + 1;
        let hb3 = MinerHeartbeatPayload {
            latest_block_hash: [0u8; 32],
            latest_height: state.block_height,
        };
        state.apply_tx(&make_tx(&sk, 4, TxType::MinerHeartbeat, &hb3)).unwrap();
        assert_eq!(state.miners.get(&addr).unwrap().reputation_bps, REPUTATION_VETERAN_BPS);
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
        });

        // At height 100 + MINER_GRACE_BLOCKS - 1: still active
        update_miner_activity(&mut state, 100 + MINER_GRACE_BLOCKS - 1);
        assert!(state.miners.get(&addr).unwrap().active);

        // At height 100 + MINER_GRACE_BLOCKS + 1: deactivated
        update_miner_activity(&mut state, 100 + MINER_GRACE_BLOCKS + 1);
        assert!(!state.miners.get(&addr).unwrap().active);
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
        state.apply_tx(&make_tx(&sk, 1, TxType::MinerRegister, &payload)).unwrap();

        let root_after = state.state_root();
        assert_ne!(root_before, root_after);
    }
}
