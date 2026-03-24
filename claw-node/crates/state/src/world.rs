//! WorldState: the complete on-chain state.

use std::collections::{BTreeMap, BTreeSet};

use borsh::{BorshDeserialize, BorshSerialize};
use claw_crypto::merkle::merkle_root;
use claw_crypto::signer::verify_transaction;
use claw_types::block::BlockEvent;
use claw_types::state::*;
use claw_types::transaction::{Transaction, TxType};

use crate::error::StateError;
use crate::handlers;

// --- Safety constants ---

/// Maximum transaction payload size (512 KB — must accommodate ContractDeploy with up to 512KB wasm).
pub const MAX_TX_PAYLOAD_SIZE: usize = 512 * 1024;

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
/// WorldState is the single source of truth for all on-chain state including
/// staking, slashing, and validator management. After the staking refactor,
/// ValidatorSet no longer maintains its own candidates — it reads from
/// WorldState.stakes directly.
#[derive(Debug, Clone, Default, BorshSerialize, BorshDeserialize)]
pub struct WorldState {
    /// Native CLAW balances.
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
    /// Current block timestamp in Unix seconds (set by the engine before applying txs).
    pub block_timestamp: u64,
    /// Deployed smart contracts.
    pub contracts: BTreeMap<[u8; 32], claw_vm::ContractInstance>,
    /// Contract storage: (contract_address, key) → value.
    pub contract_storage: BTreeMap<([u8; 32], Vec<u8>), Vec<u8>>,
    /// Contract Wasm bytecode: contract_address → code.
    pub contract_code: BTreeMap<[u8; 32], Vec<u8>>,
    /// Validator stakes: address → staked amount (in base units, 9 decimals).
    pub stakes: BTreeMap<[u8; 32], u128>,
    /// Unbonding queue for stake withdrawals awaiting the unbonding period.
    pub unbonding_queue: Vec<claw_types::state::UnbondingEntry>,
    /// Per-epoch on-chain activity statistics: address → ActivityStats.
    pub activity_stats: BTreeMap<[u8; 32], claw_types::state::ActivityStats>,
    /// Validator uptime tracking (sliding window): address → ValidatorUptime.
    pub validator_uptime: BTreeMap<[u8; 32], claw_types::state::ValidatorUptime>,
    /// Aggregated platform activity data: address → PlatformActivityAgg.
    pub platform_activity: BTreeMap<[u8; 32], claw_types::state::PlatformActivityAgg>,
    /// Tracks which Platform Agents have submitted reports this epoch: (reporter, epoch) → true.
    pub platform_report_tracker: BTreeMap<([u8; 32], u64), bool>,
    /// Stake delegation: validator_address → owner_address (who staked for them).
    /// When distributing rewards, send to owner, not validator.
    pub stake_delegations: BTreeMap<[u8; 32], [u8; 32]>,
    /// Commission rate per validator: validator_address → commission in basis points (0-10000).
    /// Default (absent) = 10000 (validator keeps all, i.e. self-stake or legacy delegation).
    pub stake_commissions: BTreeMap<[u8; 32], u16>,
    /// Token allowances: (owner, spender, token_id) → approved amount.
    pub token_allowances: BTreeMap<([u8; 32], [u8; 32], [u8; 32]), u128>,
    /// Jailed validators: address → jail_until_height.
    /// Persisted from SlashingState for state snapshot consistency.
    pub jailed_validators: BTreeMap<[u8; 32], u64>,
    /// Missed proposal slots per validator in the current epoch.
    pub validator_missed_slots: BTreeMap<[u8; 32], u64>,
    /// Total proposal slots assigned per validator in the current epoch.
    pub validator_assigned_slots: BTreeMap<[u8; 32], u64>,
    /// Permanently tracked equivocation evidence hashes (blake3 fingerprints).
    /// Prevents replay of the same evidence across restarts.
    pub processed_evidence: BTreeSet<[u8; 32]>,
    /// Registered miners: address → MinerInfo.
    pub miners: BTreeMap<[u8; 32], claw_types::state::MinerInfo>,
    /// Miner heartbeat deduplication tracker: (address, interval_window) → true.
    pub miner_heartbeat_tracker: BTreeMap<([u8; 32], u64), bool>,
}

impl WorldState {
    /// Apply a transaction to the state.
    ///
    /// Returns `(gas_fee, contract_events)` on success:
    /// - `gas_fee`: the fee charged to the sender (deducted but NOT yet credited).
    ///   Callers accumulate fees per block and call `rewards::distribute_fees`.
    /// - `contract_events`: `BlockEvent::ContractEvent` entries produced during this tx
    ///   (empty for non-contract transactions). Callers append these to the block's
    ///   event list along with reward events.
    ///
    /// The `tx_index` parameter is the 0-based position of this transaction
    /// within the current block, used to populate `BlockEvent::ContractEvent.tx_index`.
    pub fn apply_tx(
        &mut self,
        tx: &Transaction,
        tx_index: u32,
    ) -> Result<(u128, Vec<BlockEvent>), StateError> {
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

        // 3. Deduct gas (MinerHeartbeat is gas-free to encourage liveness)
        let gas_free = matches!(tx.tx_type, TxType::MinerHeartbeat);
        if !gas_free {
            let balance = self.balances.get(&tx.from).copied().unwrap_or(0);
            if balance < GAS_FEE {
                return Err(StateError::InsufficientBalance {
                    need: GAS_FEE,
                    have: balance,
                });
            }
            *self.balances.entry(tx.from).or_insert(0) -= GAS_FEE;
        }
        // Fee is deducted but not credited — caller distributes via rewards::distribute_fees

        // 4. Dispatch to handler.
        // Contract handlers return `Vec<BlockEvent>` on success; other handlers return `()`.
        // We normalise to `(Vec<BlockEvent>, Result<(), StateError>)` for uniform post-processing.
        let (contract_events, result): (Vec<BlockEvent>, Result<(), StateError>) =
            match tx.tx_type {
                TxType::ContractDeploy => {
                    match handlers::handle_contract_deploy(self, tx, tx_index) {
                        Ok(events) => (events, Ok(())),
                        Err(e) => (Vec::new(), Err(e)),
                    }
                }
                TxType::ContractCall => {
                    match handlers::handle_contract_call(self, tx, tx_index) {
                        Ok(events) => (events, Ok(())),
                        Err(e) => (Vec::new(), Err(e)),
                    }
                }
                TxType::AgentRegister => (Vec::new(), handlers::handle_agent_register(self, tx)),
                TxType::TokenTransfer => (Vec::new(), handlers::handle_token_transfer(self, tx)),
                TxType::TokenCreate => (Vec::new(), handlers::handle_token_create(self, tx)),
                TxType::TokenMintTransfer => {
                    (Vec::new(), handlers::handle_token_mint_transfer(self, tx))
                }
                TxType::ReputationAttest => {
                    (Vec::new(), handlers::handle_reputation_attest(self, tx))
                }
                TxType::ServiceRegister => {
                    (Vec::new(), handlers::handle_service_register(self, tx))
                }
                TxType::StakeDeposit => (Vec::new(), handlers::handle_stake_deposit(self, tx)),
                TxType::StakeWithdraw => (Vec::new(), handlers::handle_stake_withdraw(self, tx)),
                TxType::StakeClaim => (Vec::new(), handlers::handle_stake_claim(self, tx)),
                TxType::PlatformActivityReport => {
                    (Vec::new(), handlers::handle_platform_activity_report(self, tx))
                }
                TxType::TokenApprove => (Vec::new(), handlers::handle_token_approve(self, tx)),
                TxType::TokenBurn => (Vec::new(), handlers::handle_token_burn(self, tx)),
                TxType::ChangeDelegation => {
                    (Vec::new(), handlers::handle_change_delegation(self, tx))
                }
                TxType::MinerRegister => (Vec::new(), handlers::handle_miner_register(self, tx)),
                TxType::MinerHeartbeat => (Vec::new(), handlers::handle_miner_heartbeat(self, tx)),
                TxType::ContractUpgradeAnnounce => {
                    (Vec::new(), handlers::handle_contract_upgrade_announce(self, tx))
                }
                TxType::ContractUpgradeExecute => {
                    match handlers::handle_contract_upgrade_execute(self, tx, tx_index) {
                        Ok(events) => (events, Ok(())),
                        Err(e) => (Vec::new(), Err(e)),
                    }
                }
            };

        let actual_fee = if gas_free { 0 } else { GAS_FEE };

        if result.is_ok() {
            // 5. Update nonce on success
            self.nonces.insert(tx.from, tx.nonce);

            // 6. Update per-epoch activity stats for the sender
            let stats = self.activity_stats.entry(tx.from).or_default();
            stats.tx_count += 1;
            stats.gas_consumed += actual_fee as u64;
            match tx.tx_type {
                TxType::ContractDeploy => stats.contract_deploys += 1,
                TxType::ContractCall => stats.contract_calls += 1,
                TxType::TokenCreate => stats.tokens_created += 1,
                TxType::ServiceRegister => stats.services_registered += 1,
                _ => {}
            }

            Ok((actual_fee, contract_events))
        } else {
            // Rollback gas on failure (gas is only charged on success)
            if !gas_free {
                *self.balances.entry(tx.from).or_insert(0) += GAS_FEE;
            }
            Err(result.unwrap_err())
        }
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
            entry.extend_from_slice(&borsh::to_vec(agent).expect("borsh serialization of AgentIdentity should never fail"));
            leaves.push(*blake3::hash(&entry).as_bytes());
        }

        // Tokens
        for (id, token) in &self.tokens {
            let mut entry = Vec::new();
            entry.extend_from_slice(b"token:");
            entry.extend_from_slice(id);
            entry.extend_from_slice(&borsh::to_vec(token).expect("borsh serialization of TokenDef should never fail"));
            leaves.push(*blake3::hash(&entry).as_bytes());
        }

        // Reputation count hash (not individual records — too expensive)
        {
            let mut entry = Vec::new();
            entry.extend_from_slice(b"rep_count:");
            entry.extend_from_slice(&(self.reputation.len() as u64).to_le_bytes());
            if let Some(last) = self.reputation.last() {
                entry.extend_from_slice(&borsh::to_vec(last).expect("borsh serialization of ReputationAttestation should never fail"));
            }
            leaves.push(*blake3::hash(&entry).as_bytes());
        }

        // Services
        for ((addr, stype), svc) in &self.services {
            let mut entry = Vec::new();
            entry.extend_from_slice(b"svc:");
            entry.extend_from_slice(addr);
            entry.extend_from_slice(stype.as_bytes());
            entry.extend_from_slice(&borsh::to_vec(svc).expect("borsh serialization of ServiceEntry should never fail"));
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

        // Contract code (hash the bytecode to keep leaf size uniform)
        for (addr, code) in &self.contract_code {
            let mut entry = Vec::new();
            entry.extend_from_slice(b"ccode:");
            entry.extend_from_slice(addr);
            entry.extend_from_slice(blake3::hash(code).as_bytes());
            leaves.push(*blake3::hash(&entry).as_bytes());
        }

        // Stakes
        for (addr, amount) in &self.stakes {
            let mut entry = Vec::new();
            entry.extend_from_slice(b"stake:");
            entry.extend_from_slice(addr);
            entry.extend_from_slice(&amount.to_le_bytes());
            leaves.push(*blake3::hash(&entry).as_bytes());
        }

        // Unbonding queue
        for entry in &self.unbonding_queue {
            let mut e = Vec::new();
            e.extend_from_slice(b"unbond:");
            e.extend_from_slice(&entry.address);
            e.extend_from_slice(&entry.amount.to_le_bytes());
            e.extend_from_slice(&entry.release_height.to_le_bytes());
            leaves.push(*blake3::hash(&e).as_bytes());
        }

        // Activity stats
        for (addr, stats) in &self.activity_stats {
            let mut entry = Vec::new();
            entry.extend_from_slice(b"activity:");
            entry.extend_from_slice(addr);
            entry.extend_from_slice(&borsh::to_vec(stats).expect("borsh serialization of ActivityStats should never fail"));
            leaves.push(*blake3::hash(&entry).as_bytes());
        }

        // Validator uptime
        for (addr, uptime) in &self.validator_uptime {
            let mut entry = Vec::new();
            entry.extend_from_slice(b"uptime:");
            entry.extend_from_slice(addr);
            entry.extend_from_slice(&borsh::to_vec(uptime).expect("borsh serialization of ValidatorUptime should never fail"));
            leaves.push(*blake3::hash(&entry).as_bytes());
        }

        // Platform activity
        for (addr, agg) in &self.platform_activity {
            let mut entry = Vec::new();
            entry.extend_from_slice(b"platact:");
            entry.extend_from_slice(addr);
            entry.extend_from_slice(&borsh::to_vec(agg).expect("borsh serialization of PlatformActivityAgg should never fail"));
            leaves.push(*blake3::hash(&entry).as_bytes());
        }

        // Platform report tracker (prevents double-submission per epoch)
        for ((reporter, epoch), _) in &self.platform_report_tracker {
            let mut entry = Vec::new();
            entry.extend_from_slice(b"plattrack:");
            entry.extend_from_slice(reporter);
            entry.extend_from_slice(&epoch.to_le_bytes());
            leaves.push(*blake3::hash(&entry).as_bytes());
        }

        // Stake delegations
        for (validator, owner) in &self.stake_delegations {
            let mut entry = Vec::new();
            entry.extend_from_slice(b"deleg:");
            entry.extend_from_slice(validator);
            entry.extend_from_slice(owner);
            leaves.push(*blake3::hash(&entry).as_bytes());
        }

        // Stake commissions
        for (validator, commission_bps) in &self.stake_commissions {
            let mut entry = Vec::new();
            entry.extend_from_slice(b"commission:");
            entry.extend_from_slice(validator);
            entry.extend_from_slice(&commission_bps.to_le_bytes());
            leaves.push(*blake3::hash(&entry).as_bytes());
        }

        // Token allowances
        for ((owner, spender, token_id), amount) in &self.token_allowances {
            let mut entry = Vec::new();
            entry.extend_from_slice(b"tallowance:");
            entry.extend_from_slice(owner);
            entry.extend_from_slice(spender);
            entry.extend_from_slice(token_id);
            entry.extend_from_slice(&amount.to_le_bytes());
            leaves.push(*blake3::hash(&entry).as_bytes());
        }

        // Jailed validators
        for (addr, jail_until) in &self.jailed_validators {
            let mut entry = Vec::new();
            entry.extend_from_slice(b"jailed:");
            entry.extend_from_slice(addr);
            entry.extend_from_slice(&jail_until.to_le_bytes());
            leaves.push(*blake3::hash(&entry).as_bytes());
        }

        // Validator missed slots
        for (addr, missed) in &self.validator_missed_slots {
            let mut entry = Vec::new();
            entry.extend_from_slice(b"missed:");
            entry.extend_from_slice(addr);
            entry.extend_from_slice(&missed.to_le_bytes());
            leaves.push(*blake3::hash(&entry).as_bytes());
        }

        // Validator assigned slots
        for (addr, assigned) in &self.validator_assigned_slots {
            let mut entry = Vec::new();
            entry.extend_from_slice(b"assigned:");
            entry.extend_from_slice(addr);
            entry.extend_from_slice(&assigned.to_le_bytes());
            leaves.push(*blake3::hash(&entry).as_bytes());
        }

        // Processed equivocation evidence (permanent dedup)
        for hash in &self.processed_evidence {
            let mut entry = Vec::new();
            entry.extend_from_slice(b"evidence:");
            entry.extend_from_slice(hash);
            leaves.push(*blake3::hash(&entry).as_bytes());
        }

        // Miners
        for (addr, miner) in &self.miners {
            let mut entry = Vec::new();
            entry.extend_from_slice(b"miner:");
            entry.extend_from_slice(addr);
            entry.extend_from_slice(&borsh::to_vec(miner).expect("borsh serialization of MinerInfo should never fail"));
            leaves.push(*blake3::hash(&entry).as_bytes());
        }

        // Miner heartbeat tracker
        for ((addr, window), _) in &self.miner_heartbeat_tracker {
            let mut entry = Vec::new();
            entry.extend_from_slice(b"minerbeat:");
            entry.extend_from_slice(addr);
            entry.extend_from_slice(&window.to_le_bytes());
            leaves.push(*blake3::hash(&entry).as_bytes());
        }

        leaves.sort();
        merkle_root(&leaves)
    }

    /// Calculate total supply: sum of all balances + all stakes + unbonding.
    /// Used for supply integrity auditing.
    pub fn total_supply(&self) -> u128 {
        let total_balances: u128 = self.balances.values().sum();
        let total_stakes: u128 = self.stakes.values().sum();
        let total_unbonding: u128 = self.unbonding_queue.iter().map(|e| e.amount).sum();
        total_balances + total_stakes + total_unbonding
    }

    /// Get CLAW balance for an address.
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

    /// Get token allowance for (owner, spender, token_id).
    pub fn get_token_allowance(
        &self,
        owner: &[u8; 32],
        spender: &[u8; 32],
        token_id: &[u8; 32],
    ) -> u128 {
        self.token_allowances
            .get(&(*owner, *spender, *token_id))
            .copied()
            .unwrap_or(0)
    }
}

impl claw_vm::ChainState for WorldState {
    fn get_balance(&self, address: &[u8; 32]) -> u128 {
        self.balances.get(address).copied().unwrap_or(0)
    }

    fn get_agent_score(&self, address: &[u8; 32]) -> u64 {
        // Use the new multi-dimensional scoring if activity data exists,
        // otherwise fall back to legacy reputation sum for backward compat.
        let has_activity_data = self.activity_stats.contains_key(address)
            || self.validator_uptime.contains_key(address)
            || self.platform_activity.contains_key(address);

        if has_activity_data {
            let scores = crate::score::compute_agent_score(self, address);
            (scores.total / 100).min(100) // Scale 0-10000 to 0-100 for VM interface
        } else {
            self.reputation
                .iter()
                .filter(|r| r.to == *address)
                .map(|r| r.score as u64)
                .sum::<u64>()
                .min(100)
        }
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
