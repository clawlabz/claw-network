//! Chain: block production loop + state management + P2P integration.

use std::path::Path;
use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use claw_consensus::{elect_proposer, elect_fallback_proposer, quorum, SlashingState, ValidatorSet, BLOCK_TIME_SECS, MIN_STAKE};
use claw_crypto::ed25519_dalek::{Signature, SigningKey, Signer, VerifyingKey};
use claw_p2p::{BlockVote, NetworkEvent, P2pCommand, P2pNetwork, SyncRequest, SyncResponse};
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
    /// Slashing state: jailed validators, evidence, missed slots.
    slashing: SlashingState,
    /// Pending votes for the latest block, keyed by voter address.
    pending_votes: std::collections::HashMap<[u8; 32], [u8; 64]>,
    /// Whether to use fast sync (state snapshot) on first peer connection.
    fast_sync_pending: bool,
    /// Bootstrap peer IDs (string representation) — fast sync only targets these.
    bootstrap_peer_ids: Vec<String>,
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

        let (mut state, latest_block, validator_stakes) = match store.get_latest_height()? {
            Some(height) => {
                let state_bytes = store
                    .get_state_snapshot()?
                    .expect("snapshot must exist if blocks exist");
                let state: WorldState = borsh::from_slice(&state_bytes)?;
                let block = store.get_block(height)?.expect("block must exist");
                tracing::info!(height, "Loaded chain from storage");

                // Rebuild validator stakes from genesis config
                let stakes = genesis::build_validator_set(genesis_config)?;
                (state, block, stakes)
            }
            None => {
                let state = genesis::create_genesis_state(genesis_config)?;
                let block = genesis::create_genesis_block(&state, genesis_config);
                store.put_block(&block)?;
                store.put_state_snapshot(&borsh::to_vec(&state)?)?;
                tracing::info!(
                    chain_id = %genesis_config.chain_id,
                    allocations = genesis_config.allocations.len(),
                    validators = genesis_config.validators.len(),
                    "Created genesis block from config"
                );

                let stakes = genesis::build_validator_set(genesis_config)?;
                (state, block, stakes)
            }
        };

        // Initialize validator set from genesis config.
        // If the node's own address is not already in the validator set (devnet),
        // include it as a fallback so the node can produce blocks.
        let mut stakes = validator_stakes;
        if !stakes.iter().any(|(addr, _)| *addr == validator_address) {
            stakes.push((validator_address, MIN_STAKE * 100));
            tracing::info!(
                address = %hex::encode(validator_address),
                "Node address not in genesis validators, adding as fallback"
            );
        }
        // Write genesis stakes into WorldState so they survive epoch recalculation.
        // ValidatorSet.recalculate_active reads from its own candidates (set via
        // with_initial_stakes), but state.stakes must also be populated for
        // consistency and for the staking RPC queries.
        for (addr, amount) in &stakes {
            state.stakes.entry(*addr).or_insert(*amount);
        }
        let validator_set = ValidatorSet::with_initial_stakes(stakes);

        Ok(Self {
            inner: Arc::new(Mutex::new(ChainInner {
                state,
                store,
                mempool: Vec::new(),
                signing_key,
                validator_address,
                latest_block,
                validator_set,
                slashing: SlashingState::new(),
                pending_votes: std::collections::HashMap::new(),
                fast_sync_pending: false,
                bootstrap_peer_ids: Vec::new(),
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
    fn is_fallback_proposer(inner: &ChainInner) -> bool {
        let next_height = inner.latest_block.height + 1;
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let elapsed = now.saturating_sub(inner.latest_block.timestamp);

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
                // Update validator uptime: this validator signed a block
                let uptime = inner.state.validator_uptime.entry(vote.voter).or_default();
                uptime.signed_blocks += 1;
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

        let mut included_txs = Vec::new();
        let mut total_fees: u128 = 0;
        let mut pending = std::mem::take(&mut inner.mempool);

        // Sort by (from, nonce) for deterministic ordering
        pending.sort_by(|a, b| a.from.cmp(&b.from).then(a.nonce.cmp(&b.nonce)));

        for tx in pending {
            match inner.state.apply_tx(&tx) {
                Ok(fee) => {
                    total_fees += fee;
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

        // Distribute block rewards to validators (even for empty blocks)
        let validators: Vec<([u8; 32], u64)> = inner
            .validator_set
            .active
            .iter()
            .map(|v| (v.address, v.weight))
            .collect();
        let reward_events = claw_state::rewards::distribute_block_reward(
            &mut inner.state,
            &validators,
            new_height,
        );

        // Distribute accumulated transaction fees
        let fee_events = claw_state::rewards::distribute_fees(
            &mut inner.state,
            &inner.validator_address,
            total_fees,
        );

        // Collect all block events
        let block_events = [reward_events, fee_events].concat();

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

        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();

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

        // Persist
        if let Err(e) = inner.store.put_block(&block) {
            tracing::error!(error = %e, "Failed to store block");
            return None;
        }
        match borsh::to_vec(&inner.state) {
            Ok(state_bytes) => {
                if let Err(e) = inner.store.put_state_snapshot(&state_bytes) {
                    tracing::error!(error = %e, "Failed to store state snapshot");
                }
            }
            Err(e) => {
                tracing::error!("State serialization failed: {e}");
            }
        }

        // Sync validator set candidates from world state stakes
        Self::sync_validator_stakes(inner);

        // Check epoch boundary for validator set rotation with slashing
        if ValidatorSet::is_epoch_boundary(new_height) {
            // Process downtime slashing before recalculating active set
            {
                let mut slashing = std::mem::take(&mut inner.slashing);
                slashing.process_downtime_slashing(&mut inner.validator_set, new_height);
                inner.slashing = slashing;
            }
            inner.slashing.unjail_expired(new_height);
            let rep = inner.state.reputation.clone();
            let slashing_ref = inner.slashing.clone();
            inner.validator_set.recalculate_active_with_slashing(
                &rep,
                Some(&slashing_ref),
                new_height,
            );
            inner.slashing.reset_epoch_counters();
            tracing::info!(
                epoch = inner.validator_set.epoch,
                validators = inner.validator_set.active.len(),
                "Epoch rotation"
            );
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
        Some(block)
    }

    /// Apply a block received from the network.
    pub fn apply_remote_block(&self, block: &Block) -> Result<(), String> {
        let mut inner = self.inner.lock().expect("chain state mutex poisoned");

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

            // Always require proper BFT quorum (> 2/3)
            let required = quorum(active.len());
            if valid_signers.len() < required {
                return Err(format!(
                    "Insufficient signatures: {} of {} required",
                    valid_signers.len(),
                    required
                ));
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
        let mut total_fees: u128 = 0;
        for tx in &block.transactions {
            let fee = state_clone
                .apply_tx(tx)
                .map_err(|e| format!("tx failed: {e}"))?;
            total_fees += fee;
        }

        // Distribute block rewards to validators
        // (return values ignored — remote block already carries events)
        let validators: Vec<([u8; 32], u64)> = inner
            .validator_set
            .active
            .iter()
            .map(|v| (v.address, v.weight))
            .collect();
        let _ = claw_state::rewards::distribute_block_reward(
            &mut state_clone,
            &validators,
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

            // Signers on this block get signed_blocks incremented
            for (signer, _) in &block.signatures {
                if *signer != proposer {
                    let u = state_clone.validator_uptime.entry(*signer).or_default();
                    u.signed_blocks += 1;
                }
            }
        }

        // Verify state root
        let computed_root = state_clone.state_root();
        if computed_root != block.state_root {
            return Err("state_root mismatch".into());
        }

        // Accept the block
        inner.state = state_clone;
        if let Err(e) = inner.store.put_block(block) {
            return Err(format!("store block: {e}"));
        }
        match borsh::to_vec(&inner.state) {
            Ok(state_bytes) => {
                if let Err(e) = inner.store.put_state_snapshot(&state_bytes) {
                    tracing::error!(error = %e, "Failed to store state snapshot");
                }
            }
            Err(e) => {
                tracing::error!("State serialization failed: {e}");
            }
        }

        // Sync validator set candidates from world state stakes
        Self::sync_validator_stakes(&mut inner);

        // Epoch rotation check with slashing
        if ValidatorSet::is_epoch_boundary(block.height) {
            // Process downtime slashing before recalculating active set
            {
                let mut slashing = std::mem::take(&mut inner.slashing);
                slashing.process_downtime_slashing(&mut inner.validator_set, block.height);
                inner.slashing = slashing;
            }
            inner.slashing.unjail_expired(block.height);
            let rep = inner.state.reputation.clone();
            let slashing_ref = inner.slashing.clone();
            inner.validator_set.recalculate_active_with_slashing(
                &rep,
                Some(&slashing_ref),
                block.height,
            );
            inner.slashing.reset_epoch_counters();
        }

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
        Ok(())
    }

    /// Run the main loop: block production + P2P event processing.
    pub async fn run_with_p2p(&self, mut p2p: P2pNetwork, mut event_rx: mpsc::UnboundedReceiver<NetworkEvent>) {
        let block_interval = tokio::time::Duration::from_secs(3);
        let mut block_timer = tokio::time::interval(block_interval);

        loop {
            tokio::select! {
                _ = block_timer.tick() => {
                    // Try to produce a block
                    let maybe_block = {
                        let mut inner = self.inner.lock().expect("chain state mutex poisoned");
                        Self::produce_block(&mut inner)
                    };
                    if let Some(block) = maybe_block {
                        // Broadcast to peers
                        p2p.broadcast_block(&block);
                    }
                }
                Some(event) = event_rx.recv() => {
                    match event {
                        NetworkEvent::NewTx(tx) => {
                            // Add to mempool if valid
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
                                        p2p.broadcast_vote(&vote);
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
                            p2p.send_sync_response(channel, response);
                        }
                        NetworkEvent::SyncResponse { peer, response } => {
                            if let Some(follow_up) = self.handle_sync_response(&response) {
                                tracing::debug!(?peer, "Sending follow-up sync request");
                                p2p.send_sync_request(&peer, follow_up);
                            }
                        }
                        NetworkEvent::PeerConnected(peer) => {
                            let request = {
                                let mut inner = self.inner.lock().expect("chain state mutex poisoned");
                                if inner.fast_sync_pending {
                                    let peer_str = peer.to_string();
                                    let is_bootstrap = inner.bootstrap_peer_ids.iter().any(|id| id == &peer_str);
                                    if is_bootstrap {
                                        inner.fast_sync_pending = false;
                                        tracing::info!(%peer, "Fast sync: requesting state snapshot from bootstrap peer");
                                        SyncRequest::GetStateSnapshot
                                    } else {
                                        tracing::info!(%peer, "Fast sync pending — skipping non-bootstrap peer, sending GetStatus");
                                        SyncRequest::GetStatus
                                    }
                                } else {
                                    tracing::info!(%peer, "Peer connected — requesting chain status");
                                    SyncRequest::GetStatus
                                }
                            };
                            p2p.send_sync_request(&peer, request);
                        }
                        NetworkEvent::PeerDisconnected(peer) => {
                            tracing::info!(%peer, "Peer disconnected");
                        }
                    }
                }
            }
        }
    }

    /// Run the block production loop (single-node mode, no P2P).
    pub async fn run_block_loop(&self) {
        let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(3));
        loop {
            interval.tick().await;
            let mut inner = self.inner.lock().expect("chain state mutex poisoned");
            Self::produce_block(&mut inner);
        }
    }

    /// Process P2P network events (runs in a separate task).
    /// The `command_tx` channel sends commands back to the P2P network task.
    pub async fn run_p2p_events(
        &self,
        mut event_rx: mpsc::UnboundedReceiver<NetworkEvent>,
        command_tx: mpsc::UnboundedSender<P2pCommand>,
    ) {
        while let Some(event) = event_rx.recv().await {
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
                    tracing::info!(%peer, peers = self.get_p2p_peer_count(), "Peer disconnected");
                }
            }
        }
    }

    /// Handle a sync request from a peer.
    fn handle_sync_request(&self, request: &SyncRequest) -> SyncResponse {
        let inner = self.inner.lock().expect("chain state mutex poisoned");
        match request {
            SyncRequest::GetBlocks { from_height, count } => {
                let mut blocks = Vec::new();
                for h in *from_height..(*from_height + *count as u64) {
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
                    match self.apply_remote_block(block) {
                        Ok(()) => { applied += 1; }
                        Err(e) => {
                            // Fork detection: if the very first block fails with prev_hash mismatch,
                            // the local chain has diverged from the network. Fall back to state snapshot sync.
                            if applied == 0 && e.contains("prev_hash mismatch") {
                                tracing::warn!(
                                    height = block.height,
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
                        count: 100,
                    })
                } else {
                    None
                }
            }
            SyncResponse::Status { height } => {
                let our_height = self.get_block_number();
                if *height > our_height {
                    let count = std::cmp::min((*height - our_height) as u32, 100);
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
            SyncResponse::StateSnapshot { height, state_root, state_data, latest_block } => {
                tracing::info!(
                    snapshot_height = height,
                    state_data_size = state_data.len(),
                    "Received state snapshot from peer"
                );

                // Apply the snapshot: deserialize first, then verify state_root
                let mut inner = self.inner.lock().expect("chain state mutex poisoned");

                match borsh::from_slice::<claw_state::WorldState>(state_data) {
                    Ok(state) => {
                        // Verify state_root by recomputing from deserialized state
                        let computed_root = state.state_root();
                        if computed_root != *state_root {
                            tracing::error!(
                                expected = %hex::encode(state_root),
                                computed = %hex::encode(computed_root),
                                "State snapshot verification failed: state_root mismatch"
                            );
                            return None;
                        }

                        if let Err(e) = inner.store.put_state_snapshot(state_data) {
                            tracing::error!(error = %e, "Failed to write state snapshot to storage");
                            return None;
                        }

                        inner.state = state;
                        // Update latest_block from snapshot to re-establish chain continuity
                        inner.latest_block = latest_block.clone();
                        if let Err(e) = inner.store.put_block(latest_block) {
                            tracing::error!(error = %e, "Failed to store snapshot block");
                        }
                        tracing::info!(height, "Fork recovery: state snapshot applied, chain reset to height {}", height);

                        // Request blocks after the snapshot height to continue syncing
                        Some(SyncRequest::GetBlocks {
                            from_height: height + 1,
                            count: 100,
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
            .unwrap()
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

    /// Get validator count.
    pub fn get_validator_count(&self) -> usize {
        self.inner.lock().expect("chain state mutex poisoned").validator_set.active.len()
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
            fuel_limit: claw_vm::DEFAULT_FUEL_LIMIT,
        };

        let engine = claw_vm::VmEngine::new();
        engine
            .execute(&code, method, args, ctx, storage, &snapshot)
            .map_err(|e| e.to_string())
    }

    /// Synchronize the ValidatorSet candidates from WorldState stakes.
    /// Called after block production/application to keep the consensus-layer
    /// validator set in sync with the state-layer stake map.
    fn sync_validator_stakes(inner: &mut ChainInner) {
        let block_height = inner.state.block_height;

        // Collect addresses that are in WorldState stakes but not in ValidatorSet
        // or whose amounts differ.
        let state_stakes: Vec<([u8; 32], u128)> = inner
            .state
            .stakes
            .iter()
            .map(|(addr, amount)| (*addr, *amount))
            .collect();

        // Build a set of addresses currently staked in world state
        let staked_addrs: std::collections::HashSet<[u8; 32]> = inner
            .state
            .stakes
            .keys()
            .copied()
            .collect();

        // Remove candidates that are no longer in world state stakes
        let to_remove: Vec<[u8; 32]> = inner
            .validator_set
            .candidates
            .keys()
            .filter(|addr| !staked_addrs.contains(*addr))
            .copied()
            .collect();
        for addr in to_remove {
            inner.validator_set.candidates.remove(&addr);
        }

        // Add or update candidates from world state
        for (addr, amount) in state_stakes {
            let entry = inner
                .validator_set
                .candidates
                .entry(addr)
                .or_insert(claw_consensus::StakeInfo {
                    address: addr,
                    amount: 0,
                    staked_at: block_height,
                });
            entry.amount = amount;
        }
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
            .unwrap()
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

    /// Testnet faucet: build a real TokenTransfer transaction from the node's
    /// validator keypair to the given address and submit it to the mempool.
    /// Returns the tx hash on success.
    pub fn faucet_drip(&self, to: &[u8; 32]) -> Result<[u8; 32], String> {
        use claw_types::transaction::{TokenTransferPayload, TxType};

        let drip: u128 = 10_000_000_000; // 10 CLW (9 decimals)

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
