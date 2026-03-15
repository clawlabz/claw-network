//! WorldState: the complete on-chain state.

use std::collections::BTreeMap;

use borsh::{BorshDeserialize, BorshSerialize};
use claw_crypto::merkle::merkle_root;
use claw_crypto::signer::verify_transaction;
use claw_types::state::*;
use claw_types::transaction::{Transaction, TxType};

use crate::error::StateError;
use crate::handlers;

// --- Safety constants ---

/// Maximum transaction payload size (64 KB).
pub const MAX_TX_PAYLOAD_SIZE: usize = 64 * 1024;

/// Maximum name/service_type length (bytes).
pub const MAX_NAME_LEN: usize = 64;

/// Maximum symbol length (bytes).
pub const MAX_SYMBOL_LEN: usize = 16;

/// Maximum description length (bytes).
pub const MAX_DESCRIPTION_LEN: usize = 1024;

/// Maximum endpoint URL length (bytes).
pub const MAX_ENDPOINT_LEN: usize = 512;

/// Maximum metadata entries per agent.
pub const MAX_METADATA_ENTRIES: usize = 32;

/// Maximum memo length (bytes).
pub const MAX_MEMO_LEN: usize = 256;

/// Maximum category length (bytes).
pub const MAX_CATEGORY_LEN: usize = 64;

/// The complete world state of ClawNetwork.
///
/// Note: Custom `BorshDeserialize` is implemented to handle backward compatibility
/// with state stored before the contract fields were added. If the reader has
/// remaining bytes after `block_height`, they are parsed as contract data; otherwise
/// the contract fields default to empty.
#[derive(Debug, Clone, Default, BorshSerialize)]
pub struct WorldState {
    /// Native CLW balances.
    pub balances: BTreeMap<[u8; 32], u128>,
    /// Custom token balances: (owner, token_id) → amount.
    pub token_balances: BTreeMap<([u8; 32], [u8; 32]), u128>,
    /// Nonce per address.
    pub nonces: BTreeMap<[u8; 32], u64>,
    /// Registered agents.
    pub agents: BTreeMap<[u8; 32], AgentIdentity>,
    /// Custom token definitions.
    pub tokens: BTreeMap<[u8; 32], TokenDef>,
    /// Reputation attestations (append-only).
    pub reputation: Vec<ReputationAttestation>,
    /// Services: (provider, service_type) → entry.
    pub services: BTreeMap<([u8; 32], String), ServiceEntry>,
    /// Current block height (set by the engine before applying txs).
    pub block_height: u64,
    /// Deployed smart contracts.
    pub contracts: BTreeMap<[u8; 32], claw_vm::ContractInstance>,
    /// Contract storage: (contract_address, key) → value.
    pub contract_storage: BTreeMap<([u8; 32], Vec<u8>), Vec<u8>>,
    /// Contract Wasm bytecode: contract_address → code.
    pub contract_code: BTreeMap<[u8; 32], Vec<u8>>,
}

impl BorshDeserialize for WorldState {
    fn deserialize_reader<R: std::io::Read>(reader: &mut R) -> std::io::Result<Self> {
        let mut buf = Vec::new();
        reader.read_to_end(&mut buf)?;
        let mut cursor = std::io::Cursor::new(&buf);

        let balances = BTreeMap::<[u8; 32], u128>::deserialize_reader(&mut cursor)?;
        let token_balances =
            BTreeMap::<([u8; 32], [u8; 32]), u128>::deserialize_reader(&mut cursor)?;
        let nonces = BTreeMap::<[u8; 32], u64>::deserialize_reader(&mut cursor)?;
        let agents = BTreeMap::<[u8; 32], AgentIdentity>::deserialize_reader(&mut cursor)?;
        let tokens = BTreeMap::<[u8; 32], TokenDef>::deserialize_reader(&mut cursor)?;
        let reputation = Vec::<ReputationAttestation>::deserialize_reader(&mut cursor)?;
        let services =
            BTreeMap::<([u8; 32], String), ServiceEntry>::deserialize_reader(&mut cursor)?;
        let block_height = u64::deserialize_reader(&mut cursor)?;

        // New fields — default to empty if no more bytes remain (backward compat)
        let has_more = (cursor.position() as usize) < buf.len();
        let contracts = if has_more {
            BTreeMap::<[u8; 32], claw_vm::ContractInstance>::deserialize_reader(&mut cursor)
                .unwrap_or_default()
        } else {
            BTreeMap::new()
        };
        let has_more = (cursor.position() as usize) < buf.len();
        let contract_storage = if has_more {
            BTreeMap::<([u8; 32], Vec<u8>), Vec<u8>>::deserialize_reader(&mut cursor)
                .unwrap_or_default()
        } else {
            BTreeMap::new()
        };
        let has_more = (cursor.position() as usize) < buf.len();
        let contract_code = if has_more {
            BTreeMap::<[u8; 32], Vec<u8>>::deserialize_reader(&mut cursor).unwrap_or_default()
        } else {
            BTreeMap::new()
        };

        Ok(WorldState {
            balances,
            token_balances,
            nonces,
            agents,
            tokens,
            reputation,
            services,
            block_height,
            contracts,
            contract_storage,
            contract_code,
        })
    }
}

impl WorldState {
    /// Apply a transaction to the state. Returns Ok(()) on success.
    pub fn apply_tx(&mut self, tx: &Transaction) -> Result<(), StateError> {
        // 0. Check payload size limit
        if tx.payload.len() > MAX_TX_PAYLOAD_SIZE {
            return Err(StateError::PayloadTooLarge {
                len: tx.payload.len(),
                max: MAX_TX_PAYLOAD_SIZE,
            });
        }

        // 1. Verify signature
        verify_transaction(tx).map_err(|_| StateError::InvalidSignature)?;

        // 2. Verify nonce
        let current_nonce = self.nonces.get(&tx.from).copied().unwrap_or(0);
        let expected = current_nonce + 1;
        if tx.nonce != expected {
            return Err(StateError::InvalidNonce {
                expected,
                got: tx.nonce,
            });
        }

        // 3. Deduct gas
        let balance = self.balances.get(&tx.from).copied().unwrap_or(0);
        if balance < GAS_FEE {
            return Err(StateError::InsufficientBalance {
                need: GAS_FEE,
                have: balance,
            });
        }
        *self.balances.entry(tx.from).or_insert(0) -= GAS_FEE;
        // Gas is burned — not credited to anyone

        // 4. Dispatch to handler
        let result = match tx.tx_type {
            TxType::AgentRegister => handlers::handle_agent_register(self, tx),
            TxType::TokenTransfer => handlers::handle_token_transfer(self, tx),
            TxType::TokenCreate => handlers::handle_token_create(self, tx),
            TxType::TokenMintTransfer => handlers::handle_token_mint_transfer(self, tx),
            TxType::ReputationAttest => handlers::handle_reputation_attest(self, tx),
            TxType::ServiceRegister => handlers::handle_service_register(self, tx),
            TxType::ContractDeploy => handlers::handle_contract_deploy(self, tx),
            TxType::ContractCall => handlers::handle_contract_call(self, tx),
        };

        if result.is_ok() {
            // 5. Update nonce on success
            self.nonces.insert(tx.from, tx.nonce);
        } else {
            // Rollback gas on failure (gas is only charged on success)
            *self.balances.entry(tx.from).or_insert(0) += GAS_FEE;
        }

        result
    }

    /// Compute the Merkle state root from all state entries.
    pub fn state_root(&self) -> [u8; 32] {
        let mut leaves: Vec<[u8; 32]> = Vec::new();

        // Balances
        for (addr, bal) in &self.balances {
            let mut entry = Vec::new();
            entry.extend_from_slice(b"bal:");
            entry.extend_from_slice(addr);
            entry.extend_from_slice(&bal.to_le_bytes());
            leaves.push(*blake3::hash(&entry).as_bytes());
        }

        // Token balances
        for ((addr, tok), bal) in &self.token_balances {
            let mut entry = Vec::new();
            entry.extend_from_slice(b"tbal:");
            entry.extend_from_slice(addr);
            entry.extend_from_slice(tok);
            entry.extend_from_slice(&bal.to_le_bytes());
            leaves.push(*blake3::hash(&entry).as_bytes());
        }

        // Nonces
        for (addr, nonce) in &self.nonces {
            let mut entry = Vec::new();
            entry.extend_from_slice(b"nonce:");
            entry.extend_from_slice(addr);
            entry.extend_from_slice(&nonce.to_le_bytes());
            leaves.push(*blake3::hash(&entry).as_bytes());
        }

        // Agents
        for (addr, agent) in &self.agents {
            let mut entry = Vec::new();
            entry.extend_from_slice(b"agent:");
            entry.extend_from_slice(addr);
            entry.extend_from_slice(&borsh::to_vec(agent).unwrap());
            leaves.push(*blake3::hash(&entry).as_bytes());
        }

        // Tokens
        for (id, token) in &self.tokens {
            let mut entry = Vec::new();
            entry.extend_from_slice(b"token:");
            entry.extend_from_slice(id);
            entry.extend_from_slice(&borsh::to_vec(token).unwrap());
            leaves.push(*blake3::hash(&entry).as_bytes());
        }

        // Reputation count hash (not individual records — too expensive)
        {
            let mut entry = Vec::new();
            entry.extend_from_slice(b"rep_count:");
            entry.extend_from_slice(&(self.reputation.len() as u64).to_le_bytes());
            if let Some(last) = self.reputation.last() {
                entry.extend_from_slice(&borsh::to_vec(last).unwrap());
            }
            leaves.push(*blake3::hash(&entry).as_bytes());
        }

        // Services
        for ((addr, stype), svc) in &self.services {
            let mut entry = Vec::new();
            entry.extend_from_slice(b"svc:");
            entry.extend_from_slice(addr);
            entry.extend_from_slice(stype.as_bytes());
            entry.extend_from_slice(&borsh::to_vec(svc).unwrap());
            leaves.push(*blake3::hash(&entry).as_bytes());
        }

        // Contracts
        for (addr, instance) in &self.contracts {
            let mut entry = Vec::new();
            entry.extend_from_slice(b"contract:");
            entry.extend_from_slice(addr);
            entry.extend_from_slice(&instance.code_hash);
            entry.extend_from_slice(&instance.creator);
            entry.extend_from_slice(&instance.deployed_at.to_le_bytes());
            leaves.push(*blake3::hash(&entry).as_bytes());
        }

        // Contract storage
        for ((addr, key), value) in &self.contract_storage {
            let mut entry = Vec::new();
            entry.extend_from_slice(b"cstore:");
            entry.extend_from_slice(addr);
            entry.extend_from_slice(key);
            entry.extend_from_slice(value);
            leaves.push(*blake3::hash(&entry).as_bytes());
        }

        leaves.sort();
        merkle_root(&leaves)
    }

    /// Get CLW balance for an address.
    pub fn get_balance(&self, addr: &[u8; 32]) -> u128 {
        self.balances.get(addr).copied().unwrap_or(0)
    }

    /// Get custom token balance.
    pub fn get_token_balance(&self, addr: &[u8; 32], token_id: &[u8; 32]) -> u128 {
        self.token_balances
            .get(&(*addr, *token_id))
            .copied()
            .unwrap_or(0)
    }

    /// Get nonce for an address.
    pub fn get_nonce(&self, addr: &[u8; 32]) -> u64 {
        self.nonces.get(addr).copied().unwrap_or(0)
    }
}

impl claw_vm::ChainState for WorldState {
    fn get_balance(&self, address: &[u8; 32]) -> u128 {
        self.balances.get(address).copied().unwrap_or(0)
    }

    fn get_agent_score(&self, address: &[u8; 32]) -> u64 {
        self.reputation
            .iter()
            .filter(|r| r.to == *address)
            .map(|r| r.score as u64)
            .sum::<u64>()
            .min(100)
    }

    fn get_agent_registered(&self, address: &[u8; 32]) -> bool {
        self.agents.contains_key(address)
    }

    fn get_contract_storage(&self, contract: &[u8; 32], key: &[u8]) -> Option<Vec<u8>> {
        self.contract_storage
            .get(&(*contract, key.to_vec()))
            .cloned()
    }
}
