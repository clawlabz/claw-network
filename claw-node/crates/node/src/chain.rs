//! Chain: block production loop + state management + P2P integration.

use std::path::Path;
use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use claw_consensus::{elect_proposer, elect_fallback_proposer, quorum, SlashingState, ValidatorSet, BLOCK_TIME_SECS};
use claw_crypto::ed25519_dalek::{Signature, SigningKey, Signer, VerifyingKey};
use claw_p2p::{BlockVote, NetworkEvent, P2pCommand, SyncRequest, SyncResponse};
use claw_state::WorldState;
use claw_storage::ChainStore;
use claw_types::block::Block;
use claw_types::transaction::Transaction;
use tokio::sync::mpsc;

use crate::genesis::{self, GenesisConfig};
use crate::metrics;

/// Maximum number of transactions allowed in the mempool.
const MAX_MEMPOOL_SIZE: usize = 10_000;

/// Shared chain state.
#[derive(Clone)]
pub struct Chain {
    inner: Arc<Mutex<ChainInner>>,
    p2p_peer_count: Arc<AtomicUsize>,
}

struct ChainInner {
    state: WorldState,
    store: ChainStore,
    mempool: Vec<Transaction>,
    signing_key: SigningKey,
    validator_address: [u8; 32],
    latest_block: Block,
    validator_set: ValidatorSet,
    /// Genesis block hash — used to verify state snapshots belong to the same chain.
    genesis_hash: [u8; 32],
    /// Slashing state: jailed validators, evidence, missed slots.
    slashing: SlashingState,
    /// Pending votes for the latest block, keyed by voter address.
    pending_votes: std::collections::HashMap<[u8; 32], [u8; 64]>,
    /// Validators identified as offline at the last epoch boundary.
    /// These are excluded from block reward distribution during the current epoch.
    offline_validators: Vec<[u8; 32]>,
    /// Whether to use fast sync (state snapshot) on first peer connection.
    fast_sync_pending: bool,
    /// Bootstrap peer IDs (string representation) — fast sync only targets these.
    bootstrap_peer_ids: Vec<String>,
    /// Monotonic timestamp of when the latest block was received/produced.
    /// Immune to system clock manipulation since `Instant` is monotonic.
    latest_block_received: Instant,
}

impl Chain {
    /// Create a new chain, loading from storage or creating genesis.
    ///
    /// Accepts a `GenesisConfig` that drives the initial state and validator set
    /// when no existing chain data is found on disk.
    pub fn new(
        data_dir: &Path,
        signing_key_bytes: [u8; 32],
        genesis_config: &GenesisConfig,
    ) -> anyhow::Result<Self> {
        let db_path = data_dir.join("chain.redb");
        let store = ChainStore::open(&db_path)?;
        let signing_key = SigningKey::from_bytes(&signing_key_bytes);
        let validator_address = signing_key.verifying_key().to_bytes();

        let (mut state, latest_block, validator_stakes, genesis_hash) = match store.get_latest_height()? {
            Some(height) => {
                let state_bytes = store
                    .get_state_snapshot()?
                    .expect("snapshot must exist if blocks exist");
                let state: WorldState = borsh::from_slice(&state_bytes)?;
                let block = store.get_block(height)?.expect("block must exist");
                tracing::info!(height, "Loaded chain from storage");

                // Verify state snapshot integrity against the latest block's state_root.
                let computed_root = state.state_root();
                if computed_root != block.state_root {
                    tracing::error!(
                        block_root = %hex::encode(block.state_root),
                        computed_root = %hex::encode(computed_root),
                        height,
                        "State snapshot does not match block state_root — data is corrupted. \
                         Delete chain.redb and restart to resync from genesis."
                    );
                    anyhow::bail!(
                        "State snapshot mismatch at height {height}: block state_root={} computed={} — \
                         delete {:?} and restart",
                        hex::encode(block.state_root),
                        hex::encode(computed_root),
                        db_path,
                    );
                }

                // Get genesis hash from block 0
                let gen_hash = store.get_block(0)?
                    .map(|b| b.hash)
                    .unwrap_or([0u8; 32]);

                // Use on-chain stakes (state.stakes) as the validator set source,
                // falling back to genesis config only if state has no stakes yet.
                let stakes: Vec<([u8; 32], u128)> = if state.stakes.is_empty() {
                    genesis::build_validator_set(genesis_config)?
                } else {
                    state.stakes.iter().map(|(addr, amount)| (*addr, *amount)).collect()
                };
                (state, block, stakes, gen_hash)
            }
            None => {
                let state = genesis::create_genesis_state(genesis_config)?;
                let block = genesis::create_genesis_block(&state, genesis_config);
                let gen_hash = block.hash;
                store.put_block(&block)?;
                store.put_state_snapshot(&borsh::to_vec(&state)?)?;
                tracing::info!(
                    chain_id = %genesis_config.chain_id,
                    allocations = genesis_config.allocations.len(),
                    validators = genesis_config.validators.len(),
                    "Created genesis block from config"
                );

                let stakes = genesis::build_validator_set(genesis_config)?;
                (state, block, stakes, gen_hash)
            }
        };

        // Filter out genesis placeholder validators, but only if real validators exist.
        // Placeholders have address = [index, 0, 0, ..., 0] — no real node behind them.
        let has_real_validators = validator_stakes.iter().any(|(addr, _)| {
            !(addr[1..].iter().all(|&b| b == 0) && addr[0] != 0)
        });
        let stakes_vec: Vec<([u8; 32], u128)> = if has_real_validators {
            validator_stakes
                .into_iter()
                .filter(|(addr, _)| {
                    let is_placeholder = addr[1..].iter().all(|&b| b == 0) && addr[0] != 0;
                    if is_placeholder {
                        tracing::info!(
                            address = %hex::encode(addr),
                            "Filtered out genesis placeholder validator"
                        );
                    }
                    !is_placeholder
                })
                .collect()
        } else {
            validator_stakes
        };
        // Write genesis stakes into WorldState — the single source of truth.
        for (addr, amount) in &stakes_vec {
            state.stakes.entry(*addr).or_insert(*amount);
        }

        // If this node's address is not in the validator set, log a warning.
        // The node will sync blocks but cannot produce until it stakes.
        if !state.stakes.contains_key(&validator_address) {
            tracing::warn!(
                address = %hex::encode(validator_address),
                "Node not in validator set — will sync only, stake to become a validator"
            );
        }

        // ValidatorSet reads directly from WorldState.stakes (single source of truth).
        let validator_set = ValidatorSet::with_initial_stakes(&state.stakes);

        // Restore slashing state from WorldState persisted fields.
        let slashing = SlashingState {
            jailed: state.jailed_validators.clone(),
            missed_slots: state.validator_missed_slots.clone(),
            assigned_slots: state.validator_assigned_slots.clone(),
            processed_evidence: state.processed_evidence.clone(),
            ..Default::default()
        };

        Ok(Self {
            inner: Arc::new(Mutex::new(ChainInner {
                state,
                store,
                mempool: Vec::new(),
                signing_key,
                validator_address,
                latest_block,
                validator_set,
                genesis_hash,
                slashing,
                pending_votes: std::collections::HashMap::new(),
                offline_validators: Vec::new(),
                fast_sync_pending: false,
                bootstrap_peer_ids: Vec::new(),
                latest_block_received: Instant::now(),
            })),
            p2p_peer_count: Arc::new(AtomicUsize::new(0)),
        })
    }

    /// Enable fast sync mode so the first peer connection requests a state
    /// snapshot instead of individual blocks.
    pub fn set_fast_sync(&self) {
        let mut inner = self.inner.lock().expect("chain state mutex poisoned");
        inner.fast_sync_pending = true;
    }

    /// Store bootstrap peer IDs so fast sync only targets trusted bootstrap
    /// peers rather than mDNS-discovered local peers that may be on forks.
    pub fn set_bootstrap_peers(&self, peer_ids: Vec<String>) {
        let mut inner = self.inner.lock().expect("chain state mutex poisoned");
        inner.bootstrap_peer_ids = peer_ids;
    }

    /// Submit a transaction to the mempool.
    pub fn submit_tx(&self, tx: Transaction) -> Result<[u8; 32], String> {
        let mut inner = self.inner.lock().expect("chain state mutex poisoned");

        // Reject if mempool is full
        if inner.mempool.len() >= MAX_MEMPOOL_SIZE {
            return Err("mempool full: try again later".into());
        }

        // Basic pre-validation (signature + nonce) without applying
        claw_crypto::signer::verify_transaction(&tx)
            .map_err(|e| format!("invalid signature: {e}"))?;

        let current_nonce = inner.state.get_nonce(&tx.from);
        if tx.nonce != current_nonce + 1 {
            return Err(format!(
                "invalid nonce: expected {}, got {}",
                current_nonce + 1,
                tx.nonce
            ));
        }

        let tx_hash = tx.hash();

        // Reject duplicate transactions already in the mempool
        if inner.mempool.iter().any(|m| m.hash() == tx_hash) {
            return Err("duplicate transaction in mempool".into());
        }

        inner.mempool.push(tx);
        metrics::MEMPOOL_SIZE.set(inner.mempool.len() as f64);
        Ok(tx_hash)
    }

    /// Check if we are the proposer for the next block.
    fn is_proposer(inner: &ChainInner) -> bool {
        let next_height = inner.latest_block.height + 1;
        match elect_proposer(
            &inner.validator_set.active,
            &inner.latest_block.hash,
            next_height,
        ) {
            Some(addr) => addr == inner.validator_address,
            None => false,
        }
    }

    /// Check if we are the fallback proposer and the primary has timed out.
    /// Returns true if elapsed time > 2x BLOCK_TIME_SECS and we are the fallback.
    ///
    /// Uses monotonic `Instant` (latest_block_received) instead of `SystemTime`
    /// to prevent clock manipulation attacks where a validator sets their clock
    /// forward to always activate as fallback.
    fn is_fallback_proposer(inner: &ChainInner) -> bool {
        let next_height = inner.latest_block.height + 1;
        let elapsed = inner.latest_block_received.elapsed().as_secs();

        // Only activate fallback after 2x block time
        if elapsed <= BLOCK_TIME_SECS * 2 {
            return false;
        }

        match elect_fallback_proposer(
            &inner.validator_set.active,
            &inner.latest_block.hash,
            next_height,
        ) {
            Some(addr) => addr == inner.validator_address,
            None => false,
        }
    }

    /// Sign a block with the given secret key and append the signature.
    fn sign_block(block: &mut Block, secret_key: &SigningKey) {
        let sig = secret_key.sign(&block.hash);
        let address = secret_key.verifying_key().to_bytes();
        block.signatures.push((address, sig.to_bytes()));
    }

    /// Create a vote for a block if we are an active validator.
    /// Returns Some(BlockVote) if we should broadcast our vote.
    fn create_vote_for_block(inner: &ChainInner, block: &Block) -> Option<BlockVote> {
        if !inner.validator_set.is_active(&inner.validator_address) {
            return None;
        }
        let sig = inner.signing_key.sign(&block.hash);
        Some(BlockVote {
            block_hash: block.hash,
            height: block.height,
            voter: inner.validator_address,
            signature: sig.to_bytes(),
        })
    }

    /// Apply an incoming vote to the latest block.
    /// Returns true if the vote was valid and accepted.
    fn apply_vote(inner: &mut ChainInner, vote: &BlockVote) -> bool {
        // Only accept votes for the latest block
        if vote.height != inner.latest_block.height || vote.block_hash != inner.latest_block.hash {
            return false;
        }
        // Check voter is an active validator
        if !inner.validator_set.is_active(&vote.voter) {
            return false;
        }
        // Skip if we already have this voter's signature
        if inner.pending_votes.contains_key(&vote.voter)
            || inner.latest_block.signatures.iter().any(|(a, _)| *a == vote.voter)
        {
            return false;
        }
        // Verify signature
        if let Ok(vk) = VerifyingKey::from_bytes(&vote.voter) {
            let sig = Signature::from_bytes(&vote.signature);
            if vk.verify_strict(&vote.block_hash, &sig).is_ok() {
                inner.pending_votes.insert(vote.voter, vote.signature);
                inner.latest_block.signatures.push((vote.voter, vote.signature));
                // NOTE: Do NOT update validator_uptime here. Vote-based uptime
                // updates are non-deterministic (arrive asynchronously) and would
                // cause state_root divergence between nodes during sync catch-up.
                // Uptime tracking is handled deterministically in produce_block /
                // apply_remote_block_inner based on block proposer only.
                // Re-persist the block with updated signatures
                if let Err(e) = inner.store.put_block(&inner.latest_block) {
                    tracing::error!(error = %e, "Failed to update block with new signature");
                }
                tracing::info!(
                    height = vote.height,
                    voter = %hex::encode(vote.voter),
                    total_sigs = inner.latest_block.signatures.len(),
                    "Accepted BFT vote"
                );
                return true;
            }
        }
        false
    }

    /// Produce a block from pending mempool transactions.
    fn produce_block(inner: &mut ChainInner) -> Option<Block> {
        // Supply integrity check: capture before state
        let supply_before = inner.state.total_supply();

        // Produce blocks even when mempool is empty — continuous block production
        // ensures liveness, epoch progression, uptime tracking, and Agent Score updates.

        // Check if we should produce (consensus election — primary or fallback)
        let is_primary = Self::is_proposer(inner);
        let is_fallback = !is_primary && Self::is_fallback_proposer(inner);

        if !is_primary && !is_fallback {
            return None;
        }

        // If we are producing as fallback, record the primary's missed slot
        if is_fallback {
            let next_height = inner.latest_block.height + 1;
            if let Some(primary) = elect_proposer(
                &inner.validator_set.active,
                &inner.latest_block.hash,
                next_height,
            ) {
                inner.slashing.record_assigned_slot(&primary);
                inner.slashing.record_missed_slot(&primary);
                tracing::warn!(
                    primary = %hex::encode(primary),
                    "Primary proposer timed out — producing block as fallback"
                );
            }
        }

        let new_height = inner.latest_block.height + 1;
        inner.state.block_height = new_height;
        // Capture the block timestamp before applying transactions so that
        // contracts calling block_timestamp() during execution receive the
        // actual current time rather than the hardcoded 0 that was previously
        // forwarded by the handler.
        let block_timestamp_now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or(std::time::Duration::ZERO)
            .as_secs();
        inner.state.block_timestamp = block_timestamp_now;

        let mut included_txs = Vec::new();
        let mut total_fees: u128 = 0;
        let mut tx_contract_events: Vec<claw_types::BlockEvent> = Vec::new();
        let mut pending = std::mem::take(&mut inner.mempool);

        // Sort by (from, nonce) for deterministic ordering
        pending.sort_by(|a, b| a.from.cmp(&b.from).then(a.nonce.cmp(&b.nonce)));

        // Cap per-block transaction count
        const MAX_BLOCK_TXS: usize = 500;
        if pending.len() > MAX_BLOCK_TXS {
            // Return excess transactions to mempool
            let excess = pending.split_off(MAX_BLOCK_TXS);
            inner.mempool = excess;
        }

        for (tx_index, tx) in pending.into_iter().enumerate() {
            match inner.state.apply_tx(&tx, tx_index as u32) {
                Ok((fee, events)) => {
                    total_fees += fee;
                    tx_contract_events.extend(events);
                    included_txs.push(tx);
                }
                Err(e) => {
                    tracing::warn!(
                        tx_hash = %hex::encode(tx.hash()),
                        error = %e,
                        "Transaction rejected"
                    );
                }
            }
        }

        // Distribute block rewards to validators (even for empty blocks).
        // Exclude validators identified as offline in the previous epoch.
        let validators: Vec<([u8; 32], u64)> = inner
            .validator_set
            .active
            .iter()
            .filter(|v| !inner.offline_validators.contains(&v.address))
            .map(|v| (v.address, v.weight))
            .collect();
        let reward_events = claw_state::rewards::distribute_block_reward(
            &mut inner.state,
            &validators,
            new_height,
        );

        // Distribute mining rewards (35% to active miners after upgrade height)
        let mining_events = claw_state::rewards::distribute_mining_rewards(
            &mut inner.state,
            new_height,
        );

        // Distribute accumulated transaction fees
        let fee_events = claw_state::rewards::distribute_fees(
            &mut inner.state,
            &inner.validator_address,
            total_fees,
        );

        // Collect all block events (contract events first, then reward/fee events)
        let block_events = [tx_contract_events, reward_events, mining_events, fee_events].concat();

        // Update validator uptime tracking (B3):
        // - The block proposer gets a produced_blocks increment
        // - All active validators get expected_blocks incremented
        // - The block proposer also gets signed_blocks (they sign their own block)
        {
            let proposer = inner.validator_address;
            let uptime = inner.state.validator_uptime.entry(proposer).or_default();
            uptime.produced_blocks += 1;
            uptime.signed_blocks += 1;
            uptime.expected_blocks += 1;

            // All other active validators get expected_blocks incremented
            let other_validators: Vec<[u8; 32]> = inner.validator_set.active.iter()
                .filter(|v| v.address != proposer)
                .map(|v| v.address)
                .collect();
            for addr in other_validators {
                let u = inner.state.validator_uptime.entry(addr).or_default();
                u.expected_blocks += 1;
            }
        }

        // Reuse the timestamp captured before tx execution so the block header
        // and the state's block_timestamp are consistent.
        let timestamp = block_timestamp_now;

        // Sync slashing state to WorldState BEFORE computing state_root
        // so the root commits to current slashing data
        Self::sync_slashing_to_world_state(inner);
        let state_root = inner.state.state_root();

        let mut block = Block {
            height: new_height,
            prev_hash: inner.latest_block.hash,
            timestamp,
            validator: inner.validator_address,
            transactions: included_txs,
            state_root,
            hash: [0u8; 32],
            signatures: Vec::new(),
            events: block_events,
        };
        block.hash = block.compute_hash();

        // Sign the block with our validator key for BFT finality
        Self::sign_block(&mut block, &inner.signing_key);

        // Clear pending votes for the new block
        inner.pending_votes.clear();

        // Supply integrity check BEFORE epoch boundary processing.
        // Slashing burns tokens, so must check before slashing runs.
        let supply_after = inner.state.total_supply();
        // Compute expected burn exactly as distribute_fees does to avoid rounding mismatch
        let proposer_share = total_fees * 50 / 100;
        let ecosystem_share = total_fees * 20 / 100;
        let expected_burn = total_fees - proposer_share - ecosystem_share;
        let expected_supply = supply_before - expected_burn;
        if supply_after != expected_supply {
            tracing::error!(
                height = block.height,
                before = supply_before,
                after = supply_after,
                expected = expected_supply,
                diff = supply_after as i128 - expected_supply as i128,
                total_fees,
                "SUPPLY INTEGRITY VIOLATION in produce_block"
            );
        }

        // Atomically persist block + state snapshot in a single transaction.
        // This prevents inconsistency on crash between two separate writes.
        Self::sync_slashing_to_world_state(inner);
        match borsh::to_vec(&inner.state) {
            Ok(state_bytes) => {
                if let Err(e) = inner.store.put_block_and_snapshot(&block, &state_bytes) {
                    tracing::error!(error = %e, "Failed to store block and snapshot");
                    return None;
                }
            }
            Err(e) => {
                tracing::error!("State serialization failed: {e}");
                return None;
            }
        }

        // Epoch boundary: validator set rotation (no downtime slashing)
        if ValidatorSet::is_epoch_boundary(new_height) {
            // Identify offline validators — excluded from rewards in the NEXT epoch.
            inner.offline_validators = inner.slashing.process_downtime_penalties();

            inner.slashing.unjail_expired(new_height);

            // Update miner activity — deactivate miners who missed heartbeats
            claw_state::rewards::update_miner_activity(&mut inner.state, new_height);

            // Recalculate active set
            let stakes = inner.state.stakes.clone();
            let rep = inner.state.reputation.clone();
            let slashing_ref = inner.slashing.clone();
            inner.validator_set.recalculate_active(
                &stakes,
                &rep,
                Some(&slashing_ref),
                new_height,
            );
            inner.slashing.reset_epoch_counters();
            // Reset uptime counters for new epoch
            inner.state.validator_uptime.clear();
            tracing::info!(
                epoch = inner.validator_set.epoch,
                validators = inner.validator_set.active.len(),
                "Epoch rotation"
            );

            // Persist slashing state changes from epoch processing
            Self::sync_slashing_to_world_state(inner);
            match borsh::to_vec(&inner.state) {
                Ok(state_bytes) => {
                    if let Err(e) = inner.store.put_state_snapshot(&state_bytes) {
                        tracing::error!(error = %e, "Failed to store post-epoch state snapshot");
                    }
                }
                Err(e) => {
                    tracing::error!("Post-epoch state serialization failed: {e}");
                }
            }
        }

        tracing::info!(
            height = block.height,
            txs = block.transactions.len(),
            "Block produced"
        );

        // Record metrics
        metrics::BLOCKS_TOTAL.inc();
        metrics::TRANSACTIONS_TOTAL.inc_by(block.transactions.len() as f64);
        metrics::BLOCK_HEIGHT.set(block.height as f64);
        metrics::MEMPOOL_SIZE.set(inner.mempool.len() as f64);
        let block_time = block.timestamp.saturating_sub(inner.latest_block.timestamp);
        if block_time > 0 {
            metrics::BLOCK_TIME_SECONDS.observe(block_time as f64);
        }

        inner.latest_block = block.clone();
        inner.latest_block_received = Instant::now();
        Some(block)
    }

    /// Apply a block received from the network (gossipsub — strict BFT).
    pub fn apply_remote_block(&self, block: &Block) -> Result<(), String> {
        self.apply_remote_block_inner(block, false)
    }

    /// Apply a block received via sync catch-up (relaxed signature check).
    pub fn apply_synced_block(&self, block: &Block) -> Result<(), String> {
        self.apply_remote_block_inner(block, true)
    }

    fn apply_remote_block_inner(&self, block: &Block, is_sync: bool) -> Result<(), String> {
        let mut inner = self.inner.lock().expect("chain state mutex poisoned");
        let supply_before = inner.state.total_supply();

        // Validate block connects to our chain
        if block.height != inner.latest_block.height + 1 {
            return Err(format!(
                "block height mismatch: expected {}, got {}",
                inner.latest_block.height + 1,
                block.height
            ));
        }
        if block.prev_hash != inner.latest_block.hash {
            return Err("prev_hash mismatch".into());
        }

        // Verify block hash
        if !block.verify_hash() {
            return Err("invalid block hash".into());
        }

        // Reject blocks with timestamps too far in the future (30s clock skew tolerance)
        let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs();
        if block.timestamp > now + 30 {
            return Err(format!("block timestamp {} is in the future", block.timestamp));
        }
        // Reject timestamp regression
        if block.timestamp < inner.latest_block.timestamp {
            return Err("block timestamp regresses".into());
        }

        // Reject oversized blocks
        if block.transactions.len() > 500 {
            return Err(format!("block has too many transactions: {}", block.transactions.len()));
        }

        // Verify BFT signatures for finality
        {
            let active = &inner.validator_set.active;
            let mut valid_signers = std::collections::HashSet::new();
            for (addr, sig_bytes) in &block.signatures {
                // Check signer is an active validator
                if active.iter().any(|v| v.address == *addr) {
                    // Verify Ed25519 signature over block hash
                    if let Ok(vk) = VerifyingKey::from_bytes(addr) {
                        let sig = Signature::from_bytes(sig_bytes);
                        if vk.verify_strict(&block.hash, &sig).is_ok() {
                            valid_signers.insert(*addr);
                        }
                    }
                }
            }

            // BFT quorum check.
            // For small networks (<7 validators), only require proposer signature
            // to avoid liveness issues when any validator is offline.
            // For mature networks (>=7), require strict BFT quorum (>2/3).
            // During sync catch-up, always accept proposer-only.
            let strict_required = quorum(active.len());
            let required = if active.len() < 7 { 1 } else { strict_required };
            if valid_signers.len() < required {
                let is_proposer_signed = valid_signers.contains(&block.validator);

                if is_sync && is_proposer_signed {
                    // Sync catch-up: accept with proposer signature only
                    tracing::debug!(
                        height = block.height,
                        signatures = valid_signers.len(),
                        required,
                        "Accepting under-signed block during sync catch-up"
                    );
                } else {
                    return Err(format!(
                        "Insufficient signatures: {} of {} required",
                        valid_signers.len(),
                        required
                    ));
                }
            }
        }

        // Verify proposer authorization (accept primary or fallback proposer)
        let expected_proposer = elect_proposer(
            &inner.validator_set.active,
            &block.prev_hash,
            block.height,
        );
        let fallback = elect_fallback_proposer(
            &inner.validator_set.active,
            &block.prev_hash,
            block.height,
        );
        if let Some(expected) = expected_proposer {
            let is_primary = block.validator == expected;
            let is_fallback = fallback.map_or(false, |fb| block.validator == fb);
            if !is_primary && !is_fallback {
                return Err(format!(
                    "Block proposer mismatch: expected {} (or fallback {}), got {}",
                    hex::encode(expected),
                    fallback.map_or_else(|| "none".to_string(), |fb| hex::encode(fb)),
                    hex::encode(block.validator)
                ));
            }
            // If the fallback produced this block, record a missed slot for the primary
            if is_fallback && !is_primary {
                inner.slashing.record_assigned_slot(&expected);
                inner.slashing.record_missed_slot(&expected);
            }
        }

        // Apply all transactions
        let mut state_clone = inner.state.clone();
        state_clone.block_height = block.height;
        // Forward the block's timestamp so contracts calling block_timestamp()
        // receive the actual block time, not the default 0.
        state_clone.block_timestamp = block.timestamp;
        let mut total_fees: u128 = 0;
        for (tx_index, tx) in block.transactions.iter().enumerate() {
            let (fee, _events) = state_clone
                .apply_tx(tx, tx_index as u32)
                .map_err(|e| format!("tx failed: {e}"))?;
            total_fees += fee;
        }

        // Distribute block rewards to validators, excluding offline ones.
        // (return values ignored — remote block already carries events)
        let validators: Vec<([u8; 32], u64)> = inner
            .validator_set
            .active
            .iter()
            .filter(|v| !inner.offline_validators.contains(&v.address))
            .map(|v| (v.address, v.weight))
            .collect();
        let _ = claw_state::rewards::distribute_block_reward(
            &mut state_clone,
            &validators,
            block.height,
        );

        // Distribute mining rewards (35% to active miners after upgrade height)
        let _ = claw_state::rewards::distribute_mining_rewards(
            &mut state_clone,
            block.height,
        );

        // Distribute accumulated transaction fees
        let _ = claw_state::rewards::distribute_fees(
            &mut state_clone,
            &block.validator,
            total_fees,
        );

        // Update validator uptime tracking for remote blocks (B3)
        {
            let proposer = block.validator;
            let uptime = state_clone.validator_uptime.entry(proposer).or_default();
            uptime.produced_blocks += 1;
            uptime.signed_blocks += 1;
            uptime.expected_blocks += 1;

            // All other active validators get expected_blocks incremented
            let other_validators: Vec<[u8; 32]> = inner.validator_set.active.iter()
                .filter(|v| v.address != proposer)
                .map(|v| v.address)
                .collect();
            for addr in other_validators {
                let u = state_clone.validator_uptime.entry(addr).or_default();
                u.expected_blocks += 1;
            }

            // NOTE: Do NOT increment signed_blocks for extra vote signers here.
            // Vote signatures on stored blocks are non-deterministic (accumulated
            // asynchronously), so including them would cause state_root divergence
            // between live nodes and syncing nodes.
        }

        // Sync slashing state to state_clone BEFORE computing state_root
        state_clone.jailed_validators = inner.slashing.jailed.clone();
        state_clone.validator_missed_slots = inner.slashing.missed_slots.clone();
        state_clone.validator_assigned_slots = inner.slashing.assigned_slots.clone();
        state_clone.processed_evidence = inner.slashing.processed_evidence.clone();

        // Verify state root
        let computed_root = state_clone.state_root();
        if computed_root != block.state_root {
            return Err("state_root mismatch".into());
        }

        // Accept the block
        inner.state = state_clone;

        // Supply integrity check BEFORE epoch boundary processing.
        // Slashing burns tokens, so we must check before slashing runs.
        let supply_after = inner.state.total_supply();
        let proposer_share = total_fees * 50 / 100;
        let ecosystem_share = total_fees * 20 / 100;
        let expected_burn = total_fees - proposer_share - ecosystem_share;
        let expected_supply = supply_before - expected_burn;
        if supply_after != expected_supply {
            tracing::error!(
                height = block.height,
                before = supply_before,
                after = supply_after,
                expected = expected_supply,
                diff = supply_after as i128 - expected_supply as i128,
                total_fees,
                "SUPPLY INTEGRITY VIOLATION in apply_remote_block"
            );
        }

        // Atomically persist block + state snapshot in a single transaction.
        Self::sync_slashing_to_world_state(&mut inner);
        match borsh::to_vec(&inner.state) {
            Ok(state_bytes) => {
                if let Err(e) = inner.store.put_block_and_snapshot(block, &state_bytes) {
                    return Err(format!("store block and snapshot: {e}"));
                }
            }
            Err(e) => {
                return Err(format!("state serialization failed: {e}"));
            }
        }

        // Epoch rotation (no downtime slashing — only reward exclusion)
        if ValidatorSet::is_epoch_boundary(block.height) {
            inner.offline_validators = inner.slashing.process_downtime_penalties();
            inner.slashing.unjail_expired(block.height);

            // Update miner activity — deactivate miners who missed heartbeats
            claw_state::rewards::update_miner_activity(&mut inner.state, block.height);

            // Recalculate active set
            let stakes = inner.state.stakes.clone();
            let rep = inner.state.reputation.clone();
            let slashing_ref = inner.slashing.clone();
            inner.validator_set.recalculate_active(
                &stakes,
                &rep,
                Some(&slashing_ref),
                block.height,
            );
            inner.slashing.reset_epoch_counters();
            // Reset uptime counters for new epoch
            inner.state.validator_uptime.clear();

            // Persist slashing state changes from epoch processing
            Self::sync_slashing_to_world_state(&mut inner);
            match borsh::to_vec(&inner.state) {
                Ok(state_bytes) => {
                    if let Err(e) = inner.store.put_state_snapshot(&state_bytes) {
                        tracing::error!(error = %e, "Failed to store post-epoch state snapshot");
                    }
                }
                Err(e) => {
                    tracing::error!("Post-epoch state serialization failed: {e}");
                }
            }
        }

        // Clear pending votes after accepting a remote block
        inner.pending_votes.clear();

        tracing::info!(
            height = block.height,
            txs = block.transactions.len(),
            "Applied remote block"
        );

        // Record metrics
        metrics::BLOCKS_TOTAL.inc();
        metrics::TRANSACTIONS_TOTAL.inc_by(block.transactions.len() as f64);
        metrics::BLOCK_HEIGHT.set(block.height as f64);

        inner.latest_block = block.clone();
        inner.latest_block_received = Instant::now();
        Ok(())
    }

    /// Run the block production loop.
    /// When `command_tx` is provided, produced blocks are broadcast via P2P gossipsub
    /// and the proposer's own vote is also broadcast.
    pub async fn run_block_loop(
        &self,
        command_tx: Option<mpsc::UnboundedSender<P2pCommand>>,
    ) {
        let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(3));
        loop {
            interval.tick().await;
            let mut inner = self.inner.lock().expect("chain state mutex poisoned");
            if let Some(block) = Self::produce_block(&mut inner) {
                if let Some(ref tx) = command_tx {
                    let _ = tx.send(P2pCommand::BroadcastBlock(block.clone()));
                    // Also broadcast our own vote for the block we just produced
                    if let Some(vote) = Self::create_vote_for_block(&inner, &block) {
                        let _ = tx.send(P2pCommand::BroadcastVote(vote));
                    }
                }
            }
        }
    }

    /// Process P2P network events (runs in a separate task).
    /// The `command_tx` channel sends commands back to the P2P network task.
    pub async fn run_p2p_events(
        &self,
        mut event_rx: mpsc::UnboundedReceiver<NetworkEvent>,
        command_tx: mpsc::UnboundedSender<P2pCommand>,
    ) {
        let mut sync_retry = tokio::time::interval(tokio::time::Duration::from_secs(15));
        let mut known_peers: Vec<claw_p2p::PeerId> = Vec::new();

        loop {
            tokio::select! {
                _ = sync_retry.tick() => {
                    // Periodic sync: ask all known peers for their status
                    if !known_peers.is_empty() {
                        let our_height = self.get_block_number();
                        for peer in &known_peers {
                            let _ = command_tx.send(P2pCommand::SendSyncRequest {
                                peer: *peer,
                                request: SyncRequest::GetStatus,
                            });
                        }
                        tracing::debug!(our_height, peers = known_peers.len(), "Sync retry: requesting status from all peers");
                    }
                }
                Some(event) = event_rx.recv() => {
            match event {
                NetworkEvent::NewTx(tx) => {
                    match self.submit_tx(tx) {
                        Ok(hash) => {
                            tracing::debug!(tx_hash = %hex::encode(hash), "Accepted tx from network");
                        }
                        Err(e) => {
                            tracing::debug!(error = %e, "Rejected tx from network");
                        }
                    }
                }
                NetworkEvent::NewBlock(block) => {
                    match self.apply_remote_block(&block) {
                        Ok(()) => {
                            // If we are a validator, sign and broadcast our vote
                            let maybe_vote = {
                                let inner = self.inner.lock().expect("chain state mutex poisoned");
                                Self::create_vote_for_block(&inner, &block)
                            };
                            if let Some(vote) = maybe_vote {
                                let _ = command_tx.send(P2pCommand::BroadcastVote(vote));
                            }
                        }
                        Err(e) => {
                            tracing::debug!(error = %e, "Rejected block from network");
                        }
                    }
                }
                NetworkEvent::Vote(vote) => {
                    let mut inner = self.inner.lock().expect("chain state mutex poisoned");
                    Self::apply_vote(&mut inner, &vote);
                }
                NetworkEvent::SyncRequest { peer, request, channel, .. } => {
                    let response = self.handle_sync_request(&request);
                    tracing::debug!(?peer, "Sync request handled, sending response");
                    let _ = command_tx.send(P2pCommand::SendSyncResponse {
                        channel,
                        response,
                    });
                }
                NetworkEvent::SyncResponse { peer, response } => {
                    if let Some(follow_up) = self.handle_sync_response(&response) {
                        tracing::debug!(?peer, "Sending follow-up sync request");
                        let _ = command_tx.send(P2pCommand::SendSyncRequest {
                            peer,
                            request: follow_up,
                        });
                    }
                }
                NetworkEvent::PeerConnected(peer) => {
                    self.peer_connected();
                    if !known_peers.contains(&peer) {
                        known_peers.push(peer);
                    }
                    let request = {
                        let mut inner = self.inner.lock().expect("chain state mutex poisoned");
                        if inner.fast_sync_pending {
                            let peer_str = peer.to_string();
                            let is_bootstrap = inner.bootstrap_peer_ids.iter().any(|id| id == &peer_str);
                            if is_bootstrap {
                                inner.fast_sync_pending = false;
                                tracing::info!(%peer, peers = self.get_p2p_peer_count(), "Fast sync: requesting state snapshot from bootstrap peer");
                                SyncRequest::GetStateSnapshot
                            } else {
                                tracing::info!(%peer, peers = self.get_p2p_peer_count(), "Fast sync pending — skipping non-bootstrap peer, sending GetStatus");
                                SyncRequest::GetStatus
                            }
                        } else {
                            tracing::info!(%peer, peers = self.get_p2p_peer_count(), "Peer connected — requesting chain status");
                            SyncRequest::GetStatus
                        }
                    };
                    let _ = command_tx.send(P2pCommand::SendSyncRequest {
                        peer,
                        request,
                    });
                }
                NetworkEvent::PeerDisconnected(peer) => {
                    self.peer_disconnected();
                    known_peers.retain(|p| p != &peer);
                    tracing::info!(%peer, peers = self.get_p2p_peer_count(), "Peer disconnected");
                }
            }
                } // close Some(event)
            } // close select!
        } // close loop
    }

    /// Handle a sync request from a peer.
    fn handle_sync_request(&self, request: &SyncRequest) -> SyncResponse {
        let inner = self.inner.lock().expect("chain state mutex poisoned");
        match request {
            SyncRequest::GetBlocks { from_height, count } => {
                let capped_count = (*count).min(100) as u64;
                let mut blocks = Vec::new();
                let end = from_height.saturating_add(capped_count);
                for h in *from_height..end {
                    if h == inner.latest_block.height {
                        blocks.push(inner.latest_block.clone());
                    } else if let Ok(Some(block)) = inner.store.get_block(h) {
                        blocks.push(block);
                    } else {
                        break;
                    }
                }
                SyncResponse::Blocks(blocks)
            }
            SyncRequest::GetStatus => SyncResponse::Status {
                height: inner.latest_block.height,
            },
            SyncRequest::GetStateSnapshot => {
                match inner.store.get_state_snapshot() {
                    Ok(Some(state_data)) => {
                        tracing::debug!(
                            height = inner.latest_block.height,
                            state_data_size = state_data.len(),
                            "Serving state snapshot to peer"
                        );
                        SyncResponse::StateSnapshot {
                            height: inner.latest_block.height,
                            state_root: inner.latest_block.state_root,
                            state_data,
                            latest_block: inner.latest_block.clone(),
                            genesis_hash: inner.genesis_hash,
                        }
                    }
                    _ => {
                        tracing::warn!("State snapshot requested but not available");
                        SyncResponse::Status { height: inner.latest_block.height }
                    }
                }
            }
        }
    }

    /// Handle a sync response from a peer.
    /// Returns an optional follow-up SyncRequest if more blocks are needed.
    fn handle_sync_response(&self, response: &SyncResponse) -> Option<SyncRequest> {
        match response {
            SyncResponse::Blocks(blocks) => {
                if blocks.is_empty() {
                    tracing::debug!("Received empty blocks response — sync complete or peer has no more blocks");
                    return None;
                }
                let batch_count = blocks.len();
                let mut applied = 0;
                for block in blocks {
                    match self.apply_synced_block(block) {
                        Ok(()) => { applied += 1; }
                        Err(e) => {
                            // Fork detection: if the very first block fails with prev_hash mismatch,
                            // the local chain has diverged from the network. Fall back to state snapshot sync.
                            if applied == 0 && (e.contains("prev_hash mismatch") || e.contains("state_root mismatch")) {
                                tracing::warn!(
                                    height = block.height,
                                    error = %e,
                                    "Fork detected: local chain diverged from network. Requesting state snapshot for recovery."
                                );
                                return Some(SyncRequest::GetStateSnapshot);
                            }
                            tracing::debug!(
                                height = block.height,
                                error = %e,
                                "Failed to apply synced block"
                            );
                            break;
                        }
                    }
                }
                tracing::info!(applied, batch_count, "Applied synced blocks");
                // If we applied the full batch, request the next batch
                if applied == batch_count {
                    let our_height = self.get_block_number();
                    Some(SyncRequest::GetBlocks {
                        from_height: our_height + 1,
                        count: 20,
                    })
                } else {
                    None
                }
            }
            SyncResponse::Status { height } => {
                let our_height = self.get_block_number();
                if *height > our_height {
                    let count = std::cmp::min((*height - our_height) as u32, 20);
                    tracing::info!(
                        our_height,
                        peer_height = height,
                        requesting_blocks = count,
                        "Peer is ahead — requesting missing blocks"
                    );
                    Some(SyncRequest::GetBlocks {
                        from_height: our_height + 1,
                        count,
                    })
                } else {
                    tracing::debug!(
                        our_height,
                        peer_height = height,
                        "Peer is at same height or behind — no sync needed"
                    );
                    None
                }
            }
            SyncResponse::StateSnapshot { height, state_root: _, state_data, latest_block, genesis_hash } => {
                tracing::info!(
                    snapshot_height = height,
                    state_data_size = state_data.len(),
                    "Received state snapshot from peer"
                );

                // Apply the snapshot: deserialize first, then verify state_root
                let mut inner = self.inner.lock().expect("chain state mutex poisoned");

                // Verify genesis hash matches — reject snapshots from different chains
                if *genesis_hash != inner.genesis_hash {
                    tracing::warn!(
                        our_genesis = %hex::encode(inner.genesis_hash),
                        peer_genesis = %hex::encode(genesis_hash),
                        "Rejecting state snapshot: genesis hash mismatch (different chain)"
                    );
                    return None;
                }

                // --- Verify the snapshot's latest_block against OUR validator set ---
                // This prevents an attacker from fabricating a block with a fake
                // proposer and self-signed state_root.

                // 1. Verify block hash is self-consistent
                if !latest_block.verify_hash() {
                    tracing::warn!("Snapshot rejected: latest_block has invalid hash");
                    return None;
                }

                // 2. Check proposer is in our CURRENT validator set (before replacement)
                let is_known_validator = inner.validator_set.active.iter()
                    .any(|v| v.address == latest_block.validator);
                if !is_known_validator {
                    tracing::warn!(
                        proposer = %hex::encode(latest_block.validator),
                        "Snapshot rejected: block proposer not in our validator set"
                    );
                    return None;
                }

                // 3. Verify ALL signatures on the block and enforce BFT quorum
                {
                    let active = &inner.validator_set.active;
                    let mut valid_signers = std::collections::HashSet::new();
                    for (addr, sig_bytes) in &latest_block.signatures {
                        if active.iter().any(|v| v.address == *addr) {
                            if let Ok(vk) = VerifyingKey::from_bytes(addr) {
                                let sig = Signature::from_bytes(sig_bytes);
                                if vk.verify_strict(&latest_block.hash, &sig).is_ok() {
                                    valid_signers.insert(*addr);
                                }
                            }
                        }
                    }

                    // Proposer must have signed
                    if !valid_signers.contains(&latest_block.validator) {
                        tracing::warn!(
                            proposer = %hex::encode(latest_block.validator),
                            "Snapshot rejected: proposer signature missing or invalid"
                        );
                        return None;
                    }

                    // BFT quorum: same rules as apply_remote_block_inner.
                    // Small networks (<7): proposer signature sufficient.
                    // Large networks (>=7): require >2/3 quorum.
                    let strict_required = quorum(active.len());
                    let required = if active.len() < 7 { 1 } else { strict_required };
                    if valid_signers.len() < required {
                        tracing::warn!(
                            height = latest_block.height,
                            valid_sigs = valid_signers.len(),
                            required,
                            "Snapshot rejected: insufficient BFT quorum on latest block"
                        );
                        return None;
                    }
                }

                match borsh::from_slice::<claw_state::WorldState>(state_data) {
                    Ok(state) => {
                        // Verify state_root matches the BLOCK's state_root (not the response field).
                        // The response's state_root could be manipulated independently.
                        let computed_root = state.state_root();
                        if computed_root != latest_block.state_root {
                            tracing::error!(
                                block_state_root = %hex::encode(latest_block.state_root),
                                computed = %hex::encode(computed_root),
                                "State snapshot verification failed: state_root does not match block"
                            );
                            return None;
                        }

                        if let Err(e) = inner.store.put_state_snapshot(state_data) {
                            tracing::error!(error = %e, "Failed to write state snapshot to storage");
                            return None;
                        }

                        // Rebuild validator set from snapshot's state.stakes
                        // (WorldState.stakes is the single source of truth)
                        inner.validator_set = ValidatorSet::with_initial_stakes(&state.stakes);
                        tracing::info!(
                            validators = inner.validator_set.active.len(),
                            "Validator set rebuilt from snapshot state.stakes"
                        );

                        // Restore slashing state from WorldState persisted fields
                        inner.slashing = SlashingState {
                            jailed: state.jailed_validators.clone(),
                            missed_slots: state.validator_missed_slots.clone(),
                            assigned_slots: state.validator_assigned_slots.clone(),
                            processed_evidence: state.processed_evidence.clone(),
                            ..Default::default()
                        };

                        inner.state = state;
                        // Update latest_block from snapshot to re-establish chain continuity
                        inner.latest_block = latest_block.clone();
                        inner.latest_block_received = Instant::now();
                        if let Err(e) = inner.store.put_block(latest_block) {
                            tracing::error!(error = %e, "Failed to store snapshot block");
                        }
                        tracing::info!(height, "Fork recovery: state snapshot applied, chain reset to height {}", height);

                        // Request blocks after the snapshot height to continue syncing
                        Some(SyncRequest::GetBlocks {
                            from_height: height + 1,
                            count: 20,
                        })
                    }
                    Err(e) => {
                        tracing::error!(error = %e, "Failed to deserialize state snapshot");
                        None
                    }
                }
            }
        }
    }

    // === Query methods for RPC ===

    pub fn get_block_number(&self) -> u64 {
        self.inner.lock().expect("chain state mutex poisoned").latest_block.height
    }

    /// Full supply audit: enumerate all balances, stakes, and unbonding.
    pub fn get_total_supply_audit(&self) -> serde_json::Value {
        let inner = self.inner.lock().expect("chain state mutex poisoned");
        let total_balances: u128 = inner.state.balances.values().sum();
        let total_stakes: u128 = inner.state.stakes.values().sum();
        let total_unbonding: u128 = inner.state.unbonding_queue.iter().map(|e| e.amount).sum();
        let balances: Vec<serde_json::Value> = inner.state.balances.iter()
            .filter(|(_, b)| **b > 0)
            .map(|(addr, b)| serde_json::json!({"address": hex::encode(addr), "balance": b.to_string()}))
            .collect();
        let stakes: Vec<serde_json::Value> = inner.state.stakes.iter()
            .filter(|(_, s)| **s > 0)
            .map(|(addr, s)| serde_json::json!({"address": hex::encode(addr), "stake": s.to_string()}))
            .collect();
        serde_json::json!({
            "totalBalances": total_balances.to_string(),
            "totalStakes": total_stakes.to_string(),
            "totalUnbonding": total_unbonding.to_string(),
            "totalSupply": (total_balances + total_stakes + total_unbonding).to_string(),
            "numBalanceEntries": inner.state.balances.len(),
            "numStakeEntries": inner.state.stakes.len(),
            "numUnbondingEntries": inner.state.unbonding_queue.len(),
            "balances": balances,
            "stakes": stakes,
        })
    }

    pub fn get_block(&self, height: u64) -> Option<Block> {
        let inner = self.inner.lock().expect("chain state mutex poisoned");
        if height == inner.latest_block.height {
            return Some(inner.latest_block.clone());
        }
        inner.store.get_block(height).ok().flatten()
    }

    pub fn get_balance(&self, addr: &[u8; 32]) -> u128 {
        self.inner.lock().expect("chain state mutex poisoned").state.get_balance(addr)
    }

    pub fn get_token_balance(&self, addr: &[u8; 32], token_id: &[u8; 32]) -> u128 {
        self.inner.lock().expect("chain state mutex poisoned").state.get_token_balance(addr, token_id)
    }

    pub fn get_nonce(&self, addr: &[u8; 32]) -> u64 {
        self.inner.lock().expect("chain state mutex poisoned").state.get_nonce(addr)
    }

    pub fn get_agent(&self, addr: &[u8; 32]) -> Option<claw_types::state::AgentIdentity> {
        self.inner.lock().expect("chain state mutex poisoned").state.agents.get(addr).cloned()
    }

    pub fn get_reputation(&self, addr: &[u8; 32]) -> Vec<claw_types::state::ReputationAttestation> {
        self.inner
            .lock()
            .expect("chain state mutex poisoned")
            .state
            .reputation
            .iter()
            .filter(|r| r.to == *addr)
            .cloned()
            .collect()
    }

    pub fn get_services(&self, service_type: Option<&str>) -> Vec<claw_types::state::ServiceEntry> {
        let inner = self.inner.lock().expect("chain state mutex poisoned");
        inner
            .state
            .services
            .values()
            .filter(|s| {
                s.active && service_type.map_or(true, |st| s.service_type == st)
            })
            .cloned()
            .collect()
    }

    pub fn get_token_info(&self, token_id: &[u8; 32]) -> Option<claw_types::state::TokenDef> {
        self.inner.lock().expect("chain state mutex poisoned").state.tokens.get(token_id).cloned()
    }

    /// Get token allowance for (owner, spender, token_id).
    pub fn get_token_allowance(&self, owner: &[u8; 32], spender: &[u8; 32], token_id: &[u8; 32]) -> u128 {
        self.inner.lock().expect("chain state mutex poisoned").state.get_token_allowance(owner, spender, token_id)
    }

    /// Get transactions involving a given address (as sender or recipient).
    /// Returns Vec of (block_height, tx_index, Transaction, block_timestamp).
    pub fn get_transactions_by_address(
        &self,
        address: &[u8; 32],
        limit: usize,
        offset: usize,
    ) -> Vec<(u64, u32, Transaction, u64)> {
        let inner = self.inner.lock().expect("chain state mutex poisoned");
        inner
            .store
            .get_transactions_by_address(address, limit, offset)
            .unwrap_or_default()
    }

    pub fn get_tx_receipt(&self, tx_hash: &[u8; 32]) -> Option<(u64, usize)> {
        let inner = self.inner.lock().expect("chain state mutex poisoned");
        if let Ok(Some(height)) = inner.store.get_tx_block_height(tx_hash) {
            if let Ok(Some(block)) = inner.store.get_block(height) {
                for (i, tx) in block.transactions.iter().enumerate() {
                    if tx.hash() == *tx_hash {
                        return Some((height, i));
                    }
                }
            }
        }
        None
    }

    /// Look up a transaction by hash and return it along with block metadata.
    pub fn get_tx_by_hash(&self, tx_hash: &[u8; 32]) -> Option<(Transaction, u64, u64)> {
        let inner = self.inner.lock().expect("chain state mutex poisoned");
        if let Ok(Some(height)) = inner.store.get_tx_block_height(tx_hash) {
            if let Ok(Some(block)) = inner.store.get_block(height) {
                for tx in &block.transactions {
                    if tx.hash() == *tx_hash {
                        return Some((tx.clone(), height, block.timestamp));
                    }
                }
            }
        }
        None
    }

    /// Get P2P connected peer count.
    pub fn get_p2p_peer_count(&self) -> usize {
        self.p2p_peer_count.load(Ordering::Relaxed)
    }

    /// Increment P2P peer count.
    pub fn peer_connected(&self) {
        self.p2p_peer_count.fetch_add(1, Ordering::Relaxed);
    }

    /// Decrement P2P peer count.
    pub fn peer_disconnected(&self) {
        // Use fetch_update to prevent underflow
        let _ = self.p2p_peer_count.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |current| {
            if current > 0 { Some(current - 1) } else { None }
        });
    }

    /// Get current epoch.
    pub fn get_epoch(&self) -> u64 {
        self.inner.lock().expect("chain state mutex poisoned").validator_set.epoch
    }

    /// Get number of pending transactions in mempool.
    pub fn get_mempool_size(&self) -> usize {
        self.inner.lock().expect("chain state mutex poisoned").mempool.len()
    }

    /// Get timestamp of the latest block.
    pub fn get_last_block_timestamp(&self) -> u64 {
        self.inner.lock().expect("chain state mutex poisoned").latest_block.timestamp
    }

    /// Get contract instance metadata by address.
    pub fn get_contract_info(&self, addr: &[u8; 32]) -> Option<claw_vm::ContractInstance> {
        self.inner.lock().expect("chain state mutex poisoned").state.contracts.get(addr).cloned()
    }

    /// Get contract storage value at a specific key.
    pub fn get_contract_storage_value(&self, addr: &[u8; 32], key: &[u8]) -> Option<Vec<u8>> {
        let inner = self.inner.lock().expect("chain state mutex poisoned");
        inner
            .state
            .contract_storage
            .get(&(*addr, key.to_vec()))
            .cloned()
    }

    /// Get contract Wasm bytecode by address.
    pub fn get_contract_code(&self, addr: &[u8; 32]) -> Option<Vec<u8>> {
        self.inner.lock().expect("chain state mutex poisoned").state.contract_code.get(addr).cloned()
    }

    /// Return all `BlockEvent::ContractEvent` entries emitted by a specific contract,
    /// optionally filtered by block range.
    ///
    /// Parameters:
    /// - `contract`: contract address to filter by
    /// - `from_block`: inclusive start height (0 = genesis)
    /// - `to_block`: inclusive end height (u64::MAX = latest)
    ///
    /// Scans stored blocks; for large ranges this may be slow — callers should
    /// apply reasonable range limits.
    pub fn get_contract_events(
        &self,
        contract: &[u8; 32],
        from_block: u64,
        to_block: u64,
    ) -> Vec<claw_types::block::BlockEvent> {
        let inner = self.inner.lock().expect("chain state mutex poisoned");
        let latest_height = inner.latest_block.height;
        let to_block = to_block.min(latest_height);

        let mut events = Vec::new();
        for height in from_block..=to_block {
            let block = if height == inner.latest_block.height {
                Some(inner.latest_block.clone())
            } else {
                inner.store.get_block(height).ok().flatten()
            };

            if let Some(block) = block {
                for event in block.events {
                    if let claw_types::block::BlockEvent::ContractEvent { contract: c, .. } = &event {
                        if c == contract {
                            events.push(event);
                        }
                    }
                }
            }
        }
        events
    }

    /// Execute a read-only contract view call (no state mutation).
    pub fn call_contract_view(
        &self,
        addr: &[u8; 32],
        method: &str,
        args: &[u8],
    ) -> Result<claw_vm::ExecutionResult, String> {
        let inner = self.inner.lock().expect("chain state mutex poisoned");

        let code = inner
            .state
            .contract_code
            .get(addr)
            .cloned()
            .ok_or_else(|| "Contract not found".to_string())?;

        // Build storage snapshot for this contract
        let storage: std::collections::BTreeMap<Vec<u8>, Vec<u8>> = inner
            .state
            .contract_storage
            .iter()
            .filter(|((a, _), _)| a == addr)
            .map(|((_, k), v)| (k.clone(), v.clone()))
            .collect();

        let block_height = inner.state.block_height;
        let block_timestamp = inner.latest_block.timestamp;

        // Create a snapshot implementing ChainState for the VM
        let snapshot = ChainStateSnapshot {
            balances: inner.state.balances.clone(),
            agents: inner.state.agents.clone(),
            contract_storage: inner.state.contract_storage.clone(),
        };

        drop(inner); // Release lock before VM execution

        let ctx = claw_vm::ExecutionContext {
            caller: [0u8; 32],
            contract_address: *addr,
            block_height,
            block_timestamp,
            value: 0,
            fuel_limit: claw_vm::VIEW_CALL_FUEL_LIMIT,
            read_only: true,
        };

        // Execute with timeout to prevent infinite-loop contracts from blocking
        let method_owned = method.to_string();
        let args_owned = args.to_vec();
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let engine = claw_vm::VmEngine::new();
            let result = engine.execute(&code, &method_owned, &args_owned, ctx, storage, &snapshot);
            let _ = tx.send(result);
        });
        rx.recv_timeout(std::time::Duration::from_secs(5))
            .unwrap_or(Err(claw_vm::VmError::ExecutionFailed(
                "contract view execution timed out after 5s".to_string(),
            )))
            .map_err(|e| e.to_string())
            .map(|mut result| {
                // View calls must not produce any state mutations.
                // Clear these defensively even though the read_only guard in
                // host functions prevents any mutations from being recorded.
                result.storage_changes.clear();
                result.transfers.clear();
                result.events.clear();
                result
            })
    }

    /// Persist slashing state to WorldState fields for snapshot consistency.
    /// Called after block production/application so that slashing state
    /// survives node restart via the state snapshot.
    fn sync_slashing_to_world_state(inner: &mut ChainInner) {
        inner.state.jailed_validators = inner.slashing.jailed.clone();
        inner.state.validator_missed_slots = inner.slashing.missed_slots.clone();
        inner.state.validator_assigned_slots = inner.slashing.assigned_slots.clone();
        inner.state.processed_evidence = inner.slashing.processed_evidence.clone();
    }

    // === Staking query methods for RPC ===

    /// Get the current stake amount for an address.
    pub fn get_stake(&self, addr: &[u8; 32]) -> u128 {
        self.inner.lock().expect("chain state mutex poisoned").state.stakes.get(addr).copied().unwrap_or(0)
    }

    /// Get unbonding entries for an address.
    pub fn get_unbonding(&self, addr: &[u8; 32]) -> Vec<claw_types::state::UnbondingEntry> {
        self.inner
            .lock()
            .expect("chain state mutex poisoned")
            .state
            .unbonding_queue
            .iter()
            .filter(|e| e.address == *addr)
            .cloned()
            .collect()
    }

    /// Get the multi-dimensional Agent Score for an address.
    pub fn get_agent_score(&self, addr: &[u8; 32]) -> claw_state::AgentScoreDetail {
        let inner = self.inner.lock().expect("chain state mutex poisoned");
        claw_state::compute_agent_score(&inner.state, addr)
    }

    /// Get the owner address for a delegated validator. Returns None if no delegation exists.
    pub fn get_stake_delegation(&self, validator: &[u8; 32]) -> Option<[u8; 32]> {
        self.inner
            .lock()
            .expect("chain state mutex poisoned")
            .state
            .stake_delegations
            .get(validator)
            .copied()
    }

    /// Get active validators with their stakes and weights.
    pub fn get_validators(&self) -> Vec<serde_json::Value> {
        let inner = self.inner.lock().expect("chain state mutex poisoned");
        inner
            .validator_set
            .active
            .iter()
            .map(|v| {
                serde_json::json!({
                    "address": hex::encode(v.address),
                    "stake": v.stake.to_string(),
                    "weight": v.weight,
                    "agentScore": v.agent_score,
                })
            })
            .collect()
    }

    /// Get comprehensive details for a single validator.
    pub fn get_validator_detail(&self, addr: &[u8; 32]) -> Option<serde_json::Value> {
        let inner = self.inner.lock().expect("chain state mutex poisoned");

        // Must be an active validator or have stake
        let stake = inner.state.stakes.get(addr).copied().unwrap_or(0);
        if stake == 0 {
            return None;
        }

        // Find weight and agent_score from active set
        let (weight, agent_score) = inner
            .validator_set
            .active
            .iter()
            .find(|v| v.address == *addr)
            .map(|v| (v.weight, v.agent_score))
            .unwrap_or((0, 0));

        let commission_bps = inner.state.stake_commissions.get(addr).copied().unwrap_or(10000);
        let delegated_by = inner.state.stake_delegations.get(addr).map(hex::encode);

        // Uptime data
        let uptime = inner.state.validator_uptime.get(addr).map(|u| {
            let uptime_pct = if u.expected_blocks > 0 {
                (u.signed_blocks as f64 / u.expected_blocks as f64) * 100.0
            } else {
                0.0
            };
            serde_json::json!({
                "produced_blocks": u.produced_blocks,
                "expected_blocks": u.expected_blocks,
                "signed_blocks": u.signed_blocks,
                "uptime_pct": (uptime_pct * 10.0).round() / 10.0,
            })
        });

        let jailed = inner.slashing.is_jailed(addr, inner.state.block_height);

        Some(serde_json::json!({
            "address": hex::encode(addr),
            "stake": stake.to_string(),
            "weight": weight,
            "agentScore": agent_score,
            "commission_bps": commission_bps,
            "delegatedBy": delegated_by,
            "uptime": uptime,
            "jailed": jailed,
        }))
    }

    // === Mining query methods for RPC ===

    /// Get miner info for a given address.
    pub fn get_miner_info(&self, addr: &[u8; 32]) -> Option<claw_types::state::MinerInfo> {
        self.inner.lock().expect("chain state mutex poisoned").state.miners.get(addr).cloned()
    }

    /// Get a paginated list of miners, optionally filtered to active only.
    pub fn get_miners(&self, active_only: bool, limit: usize, offset: usize) -> Vec<claw_types::state::MinerInfo> {
        let inner = self.inner.lock().expect("chain state mutex poisoned");
        inner.state.miners.values()
            .filter(|m| !active_only || m.active)
            .skip(offset)
            .take(limit)
            .cloned()
            .collect()
    }

    /// Get aggregate mining statistics.
    pub fn get_mining_stats(&self) -> serde_json::Value {
        let inner = self.inner.lock().expect("chain state mutex poisoned");
        let total = inner.state.miners.len();
        let active = inner.state.miners.values().filter(|m| m.active).count();
        serde_json::json!({
            "totalMiners": total,
            "activeMiners": active,
            "currentBlockReward": claw_state::rewards::reward_per_block(inner.state.block_height).to_string(),
            "miningUpgradeHeight": claw_state::rewards::MINING_UPGRADE_HEIGHT,
            "upgraded": inner.state.block_height >= claw_state::rewards::MINING_UPGRADE_HEIGHT,
        })
    }

    /// Testnet faucet: build a real TokenTransfer transaction from the node's
    /// validator keypair to the given address and submit it to the mempool.
    /// Returns the tx hash on success.
    pub fn faucet_drip(&self, to: &[u8; 32]) -> Result<[u8; 32], String> {
        use claw_types::transaction::{TokenTransferPayload, TxType};

        let drip: u128 = 10_000_000_000; // 10 CLAW (9 decimals)

        let mut inner = self.inner.lock().expect("chain state mutex poisoned");

        // Pre-check: does the node have enough balance?
        let node_addr = inner.validator_address;
        let node_bal = inner.state.get_balance(&node_addr);
        if node_bal < drip {
            return Err("faucet dry".into());
        }

        // Build the payload
        let payload = TokenTransferPayload {
            to: *to,
            amount: drip,
        };
        let payload_bytes = borsh::to_vec(&payload)
            .map_err(|e| format!("serialize payload: {e}"))?;

        // Nonce = current nonce + 1
        let nonce = inner.state.get_nonce(&node_addr) + 1;

        // Build an unsigned transaction
        let mut tx = Transaction {
            tx_type: TxType::TokenTransfer,
            from: node_addr,
            nonce,
            payload: payload_bytes,
            signature: [0u8; 64],
        };

        // Sign it with the node's keypair
        claw_crypto::signer::sign_transaction(&mut tx, &inner.signing_key);

        // Submit to mempool (skip the lock — we already hold it)
        claw_crypto::signer::verify_transaction(&tx)
            .map_err(|e| format!("invalid signature: {e}"))?;

        let tx_hash = tx.hash();
        inner.mempool.push(tx);
        metrics::MEMPOOL_SIZE.set(inner.mempool.len() as f64);

        Ok(tx_hash)
    }
}

/// Read-only snapshot of chain state for VM view calls.
///
/// Implements `claw_vm::ChainState` so the VM can query balances and
/// agent info without holding the chain lock during execution.
struct ChainStateSnapshot {
    balances: std::collections::BTreeMap<[u8; 32], u128>,
    agents: std::collections::BTreeMap<[u8; 32], claw_types::state::AgentIdentity>,
    contract_storage: std::collections::BTreeMap<([u8; 32], Vec<u8>), Vec<u8>>,
}

impl claw_vm::ChainState for ChainStateSnapshot {
    fn get_balance(&self, address: &[u8; 32]) -> u128 {
        self.balances.get(address).copied().unwrap_or(0)
    }

    fn get_agent_score(&self, _address: &[u8; 32]) -> u64 {
        // Agent score is derived from reputation attestations, not stored on AgentIdentity.
        // For view calls this returns 0; full scoring uses the reputation subsystem.
        0
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
