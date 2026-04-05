//! Serialization round-trip tests for all core types.

#[cfg(test)]
mod tests {
    use crate::block::Block;
    use crate::state::*;
    use crate::transaction::*;
    use borsh::{BorshDeserialize, BorshSerialize};
    use std::collections::BTreeMap;

    fn roundtrip<T: BorshSerialize + BorshDeserialize + PartialEq + std::fmt::Debug>(val: &T) {
        let bytes = borsh::to_vec(val).expect("serialize");
        let decoded = T::try_from_slice(&bytes).expect("deserialize");
        assert_eq!(*val, decoded);
    }

    #[test]
    fn tx_type_roundtrip() {
        for tx_type in [
            TxType::AgentRegister,
            TxType::TokenTransfer,
            TxType::TokenCreate,
            TxType::TokenMintTransfer,
            TxType::ReputationAttest,
            TxType::ServiceRegister,
        ] {
            roundtrip(&tx_type);
        }
    }

    #[test]
    fn transaction_roundtrip() {
        let tx = Transaction {
            tx_type: TxType::TokenTransfer,
            from: [1u8; 32],
            nonce: 42,
            payload: vec![10, 20, 30],
            signature: [7u8; 64],
        };
        roundtrip(&tx);
    }

    #[test]
    fn agent_register_payload_roundtrip() {
        let mut meta = BTreeMap::new();
        meta.insert("platform".to_string(), "clawarena".to_string());
        meta.insert("version".to_string(), "1.0".to_string());
        let payload = AgentRegisterPayload {
            name: "test-agent".to_string(),
            metadata: meta,
        };
        roundtrip(&payload);
    }

    #[test]
    fn token_transfer_payload_roundtrip() {
        let payload = TokenTransferPayload {
            to: [2u8; 32],
            amount: 1_000_000_000,
        };
        roundtrip(&payload);
    }

    #[test]
    fn token_create_payload_roundtrip() {
        let payload = TokenCreatePayload {
            name: "Test Token".to_string(),
            symbol: "TST".to_string(),
            decimals: 6,
            total_supply: 1_000_000_000_000,
        };
        roundtrip(&payload);
    }

    #[test]
    fn token_mint_transfer_payload_roundtrip() {
        let payload = TokenMintTransferPayload {
            token_id: [3u8; 32],
            to: [4u8; 32],
            amount: 500,
        };
        roundtrip(&payload);
    }

    #[test]
    fn reputation_attest_payload_roundtrip() {
        let payload = ReputationAttestPayload {
            to: [5u8; 32],
            category: "game".to_string(),
            score: 85,
            platform: "clawarena".to_string(),
            memo: "won 3 consecutive matches".to_string(),
        };
        roundtrip(&payload);
    }

    #[test]
    fn service_register_payload_roundtrip() {
        let payload = ServiceRegisterPayload {
            service_type: "translation".to_string(),
            description: "EN-CN translation service".to_string(),
            price_token: [0u8; 32],
            price_amount: 10_000_000,
            endpoint: "https://agent.example.com/translate".to_string(),
            active: true,
        };
        roundtrip(&payload);
    }

    #[test]
    fn block_roundtrip() {
        let block = Block {
            height: 100,
            prev_hash: [9u8; 32],
            timestamp: 1710000000,
            validator: [11u8; 32],
            transactions: vec![
                Transaction {
                    tx_type: TxType::AgentRegister,
                    from: [1u8; 32],
                    nonce: 0,
                    payload: vec![],
                    signature: [0u8; 64],
                },
            ],
            state_root: [12u8; 32],
            hash: [13u8; 32],
            signatures: Vec::new(),
            events: Vec::new(),
            checkin_witnesses: Vec::new(),
        };
        roundtrip(&block);
    }

    #[test]
    fn agent_identity_roundtrip() {
        let agent = AgentIdentity {
            address: [1u8; 32],
            name: "my-agent".to_string(),
            metadata: BTreeMap::new(),
            registered_at: 50,
        };
        roundtrip(&agent);
    }

    #[test]
    fn token_def_roundtrip() {
        let token = TokenDef {
            id: [2u8; 32],
            name: "My Token".to_string(),
            symbol: "MTK".to_string(),
            decimals: 9,
            total_supply: 1_000_000_000_000_000_000,
            issuer: [3u8; 32],
        };
        roundtrip(&token);
    }

    #[test]
    fn reputation_attestation_roundtrip() {
        let rep = ReputationAttestation {
            from: [1u8; 32],
            to: [2u8; 32],
            category: "task".to_string(),
            score: -50,
            platform: "clawmarket".to_string(),
            memo: "late delivery".to_string(),
            block_height: 1000,
        };
        roundtrip(&rep);
    }

    #[test]
    fn service_entry_roundtrip() {
        let svc = ServiceEntry {
            provider: [1u8; 32],
            service_type: "code-review".to_string(),
            description: "Automated code review".to_string(),
            price_token: [0u8; 32],
            price_amount: 5_000_000,
            endpoint: "https://agent.example.com/review".to_string(),
            active: true,
        };
        roundtrip(&svc);
    }

    #[test]
    fn tx_hash_deterministic() {
        let tx = Transaction {
            tx_type: TxType::TokenTransfer,
            from: [1u8; 32],
            nonce: 1,
            payload: vec![1, 2, 3],
            signature: [0u8; 64],
        };
        let h1 = tx.hash();
        let h2 = tx.hash();
        assert_eq!(h1, h2);
        assert_ne!(h1, [0u8; 32]);
    }

    #[test]
    fn block_hash_verify() {
        let mut block = Block {
            height: 1,
            prev_hash: [0u8; 32],
            timestamp: 1710000000,
            validator: [1u8; 32],
            transactions: vec![],
            state_root: [2u8; 32],
            hash: [0u8; 32],
            signatures: Vec::new(),
            events: Vec::new(),
            checkin_witnesses: Vec::new(),
        };
        block.hash = block.compute_hash();
        assert!(block.verify_hash());

        // Tamper
        block.height = 2;
        assert!(!block.verify_hash());
    }

    #[test]
    fn tx_type_discriminant_values() {
        assert_eq!(TxType::MinerRegister as u8, 15);
        assert_eq!(TxType::MinerHeartbeat as u8, 16);
    }

    #[test]
    fn miner_register_payload_roundtrip() {
        let payload = MinerRegisterPayload {
            tier: 1,
            ip_addr: vec![192, 168, 1, 100],
            name: "my-miner".to_string(),
        };
        roundtrip(&payload);
    }

    #[test]
    fn miner_heartbeat_payload_roundtrip() {
        let payload = MinerHeartbeatPayload {
            latest_block_hash: [0xab; 32],
            latest_height: 12345,
        };
        roundtrip(&payload);
    }

    #[test]
    fn miner_tier_roundtrip() {
        let tier = MinerTier::Online;
        roundtrip(&tier);
    }

    #[test]
    fn miner_info_roundtrip() {
        let info = MinerInfo {
            address: [1u8; 32],
            tier: MinerTier::Online,
            name: "test-miner".to_string(),
            registered_at: 100,
            last_heartbeat: 200,
            ip_prefix: vec![192, 168, 1],
            active: true,
            reputation_bps: 2000,
            pending_rewards: 0,
            pending_epoch: 0,
            epoch_attendance: 0,
            consecutive_misses: 0,
            last_checkin_epoch: 0,
        };
        roundtrip(&info);
    }

    #[test]
    fn signable_bytes_excludes_signature() {
        let tx1 = Transaction {
            tx_type: TxType::TokenTransfer,
            from: [1u8; 32],
            nonce: 1,
            payload: vec![1, 2, 3],
            signature: [0u8; 64],
        };
        let tx2 = Transaction {
            signature: [255u8; 64],
            ..tx1.clone()
        };
        assert_eq!(tx1.signable_bytes(), tx2.signable_bytes());
    }
}
