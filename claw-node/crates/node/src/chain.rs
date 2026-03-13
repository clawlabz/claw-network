//! Chain: block production loop + state management + P2P integration.

use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use claw_consensus::{elect_proposer, ValidatorSet, MIN_STAKE};
use claw_crypto::ed25519_dalek::SigningKey;
use claw_p2p::{NetworkEvent, P2pNetwork, SyncRequest, SyncResponse};
use claw_state::WorldState;
use claw_storage::ChainStore;
use claw_types::block::Block;
use claw_types::transaction::Transaction;
use tokio::sync::mpsc;

use crate::genesis;
use crate::metrics;

/// Shared chain state.
#[derive(Clone)]
pub struct Chain {
    inner: Arc<Mutex<ChainInner>>,
}

struct ChainInner {
    state: WorldState,
    store: ChainStore,
    mempool: Vec<Transaction>,
    #[allow(dead_code)]
    signing_key: SigningKey,
    validator_address: [u8; 32],
    latest_block: Block,
    validator_set: ValidatorSet,
}

impl Chain {
    /// Create a new chain, loading from storage or creating genesis.
    pub fn new(data_dir: &Path, signing_key_bytes: [u8; 32]) -> anyhow::Result<Self> {
        let db_path = data_dir.join("chain.redb");
        let store = ChainStore::open(&db_path)?;
        let signing_key = SigningKey::from_bytes(&signing_key_bytes);
        let validator_address = signing_key.verifying_key().to_bytes();

        let (state, latest_block) = match store.get_latest_height()? {
            Some(height) => {
                let state_bytes = store
                    .get_state_snapshot()?
                    .expect("snapshot must exist if blocks exist");
                let state: WorldState = borsh::from_slice(&state_bytes)?;
                let block = store.get_block(height)?.expect("block must exist");
                tracing::info!(height, "Loaded chain from storage");
                (state, block)
            }
            None => {
                let mut state = genesis::create_genesis_state();
                // In testnet/single-node mode, give the node's own address some CLW
                // so it can act as a faucet for testing.
                let faucet_amount: u128 = 1_000_000_000_000_000; // 1M CLW
                *state.balances.entry(validator_address).or_insert(0) += faucet_amount;
                tracing::info!(
                    address = %hex::encode(validator_address),
                    amount = faucet_amount,
                    "Allocated testnet faucet balance to node"
                );
                let block = genesis::create_genesis_block(&state);
                store.put_block(&block)?;
                store.put_state_snapshot(&borsh::to_vec(&state)?)?;
                tracing::info!("Created genesis block");
                (state, block)
            }
        };

        // Initialize validator set — in single-node mode, self is the sole validator.
        // In multi-node mode, this would be loaded from genesis config.
        let validator_set = ValidatorSet::with_initial_stakes(vec![
            (validator_address, MIN_STAKE * 100),
        ]);

        Ok(Self {
            inner: Arc::new(Mutex::new(ChainInner {
                state,
                store,
                mempool: Vec::new(),
                signing_key,
                validator_address,
                latest_block,
                validator_set,
            })),
        })
    }

    /// Submit a transaction to the mempool.
    pub fn submit_tx(&self, tx: Transaction) -> Result<[u8; 32], String> {
        let mut inner = self.inner.lock().unwrap();

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

    /// Produce a block from pending mempool transactions.
    fn produce_block(inner: &mut ChainInner) -> Option<Block> {
        if inner.mempool.is_empty() {
            return None;
        }

        // Check if we should produce (consensus election)
        if !Self::is_proposer(inner) {
            return None;
        }

        let new_height = inner.latest_block.height + 1;
        inner.state.block_height = new_height;

        let mut included_txs = Vec::new();
        let mut pending = std::mem::take(&mut inner.mempool);

        // Sort by (from, nonce) for deterministic ordering
        pending.sort_by(|a, b| a.from.cmp(&b.from).then(a.nonce.cmp(&b.nonce)));

        for tx in pending {
            match inner.state.apply_tx(&tx) {
                Ok(()) => {
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

        if included_txs.is_empty() {
            return None;
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
        };
        block.hash = block.compute_hash();

        // Persist
        if let Err(e) = inner.store.put_block(&block) {
            tracing::error!(error = %e, "Failed to store block");
            return None;
        }
        if let Err(e) = inner
            .store
            .put_state_snapshot(&borsh::to_vec(&inner.state).unwrap())
        {
            tracing::error!(error = %e, "Failed to store state snapshot");
        }

        // Check epoch boundary for validator set rotation
        if ValidatorSet::is_epoch_boundary(new_height) {
            let rep = inner.state.reputation.clone();
            inner.validator_set.recalculate_active(&rep);
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
        let mut inner = self.inner.lock().unwrap();

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

        // Apply all transactions
        let mut state_clone = inner.state.clone();
        state_clone.block_height = block.height;
        for tx in &block.transactions {
            state_clone
                .apply_tx(tx)
                .map_err(|e| format!("tx failed: {e}"))?;
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
        if let Err(e) = inner
            .store
            .put_state_snapshot(&borsh::to_vec(&inner.state).unwrap())
        {
            tracing::error!(error = %e, "Failed to store state snapshot");
        }

        // Epoch rotation check
        if ValidatorSet::is_epoch_boundary(block.height) {
            let rep = inner.state.reputation.clone();
            inner.validator_set.recalculate_active(&rep);
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
                        let mut inner = self.inner.lock().unwrap();
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
                                Ok(()) => {}
                                Err(e) => {
                                    tracing::debug!(error = %e, "Rejected block from network");
                                }
                            }
                        }
                        NetworkEvent::SyncRequest { peer, request, .. } => {
                            let response = self.handle_sync_request(&request);
                            // Note: In a full implementation, we'd send the response back
                            // via the channel. For now we log it.
                            tracing::debug!(?peer, "Sync request handled");
                            let _ = response;
                        }
                        NetworkEvent::SyncResponse { peer, response } => {
                            self.handle_sync_response(&response);
                            tracing::debug!(?peer, "Sync response processed");
                        }
                        NetworkEvent::PeerConnected(peer) => {
                            tracing::info!(%peer, "Peer connected");
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
            let mut inner = self.inner.lock().unwrap();
            Self::produce_block(&mut inner);
        }
    }

    /// Process P2P network events (runs in a separate task).
    pub async fn run_p2p_events(&self, mut event_rx: mpsc::UnboundedReceiver<NetworkEvent>) {
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
                        Ok(()) => {}
                        Err(e) => {
                            tracing::debug!(error = %e, "Rejected block from network");
                        }
                    }
                }
                NetworkEvent::SyncRequest { peer, request, .. } => {
                    let response = self.handle_sync_request(&request);
                    tracing::debug!(?peer, "Sync request handled");
                    let _ = response;
                }
                NetworkEvent::SyncResponse { peer, response } => {
                    self.handle_sync_response(&response);
                    tracing::debug!(?peer, "Sync response processed");
                }
                NetworkEvent::PeerConnected(peer) => {
                    tracing::info!(%peer, "Peer connected");
                }
                NetworkEvent::PeerDisconnected(peer) => {
                    tracing::info!(%peer, "Peer disconnected");
                }
            }
        }
    }

    /// Handle a sync request from a peer.
    fn handle_sync_request(&self, request: &SyncRequest) -> SyncResponse {
        let inner = self.inner.lock().unwrap();
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
        }
    }

    /// Handle a sync response from a peer.
    fn handle_sync_response(&self, response: &SyncResponse) {
        match response {
            SyncResponse::Blocks(blocks) => {
                for block in blocks {
                    if let Err(e) = self.apply_remote_block(block) {
                        tracing::debug!(
                            height = block.height,
                            error = %e,
                            "Failed to apply synced block"
                        );
                        break;
                    }
                }
            }
            SyncResponse::Status { height } => {
                let our_height = self.get_block_number();
                if *height > our_height {
                    tracing::info!(
                        our_height,
                        peer_height = height,
                        "Peer is ahead — need sync"
                    );
                    // TODO: trigger sync request for missing blocks
                }
            }
        }
    }

    // === Query methods for RPC ===

    pub fn get_block_number(&self) -> u64 {
        self.inner.lock().unwrap().latest_block.height
    }

    pub fn get_block(&self, height: u64) -> Option<Block> {
        let inner = self.inner.lock().unwrap();
        if height == inner.latest_block.height {
            return Some(inner.latest_block.clone());
        }
        inner.store.get_block(height).ok().flatten()
    }

    pub fn get_balance(&self, addr: &[u8; 32]) -> u128 {
        self.inner.lock().unwrap().state.get_balance(addr)
    }

    pub fn get_token_balance(&self, addr: &[u8; 32], token_id: &[u8; 32]) -> u128 {
        self.inner.lock().unwrap().state.get_token_balance(addr, token_id)
    }

    pub fn get_nonce(&self, addr: &[u8; 32]) -> u64 {
        self.inner.lock().unwrap().state.get_nonce(addr)
    }

    pub fn get_agent(&self, addr: &[u8; 32]) -> Option<claw_types::state::AgentIdentity> {
        self.inner.lock().unwrap().state.agents.get(addr).cloned()
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
        let inner = self.inner.lock().unwrap();
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
        self.inner.lock().unwrap().state.tokens.get(token_id).cloned()
    }

    pub fn get_tx_receipt(&self, tx_hash: &[u8; 32]) -> Option<(u64, usize)> {
        let inner = self.inner.lock().unwrap();
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

    /// Get peer count from P2P network info.
    pub fn get_validator_count(&self) -> usize {
        self.inner.lock().unwrap().validator_set.active.len()
    }

    /// Get current epoch.
    pub fn get_epoch(&self) -> u64 {
        self.inner.lock().unwrap().validator_set.epoch
    }

    /// Get number of pending transactions in mempool.
    pub fn get_mempool_size(&self) -> usize {
        self.inner.lock().unwrap().mempool.len()
    }

    /// Get timestamp of the latest block.
    pub fn get_last_block_timestamp(&self) -> u64 {
        self.inner.lock().unwrap().latest_block.timestamp
    }

    /// Testnet faucet: give 10 CLW to an address from the node's balance.
    pub fn faucet_drip(&self, to: &[u8; 32]) -> Result<u128, String> {
        let mut inner = self.inner.lock().unwrap();
        let drip: u128 = 10_000_000_000; // 10 CLW (9 decimals)
        let node_addr = inner.validator_address;
        let node_bal = inner.state.get_balance(&node_addr);
        if node_bal < drip {
            return Err("faucet dry".into());
        }
        // Deduct from node, credit to recipient
        *inner.state.balances.entry(node_addr).or_insert(0) -= drip;
        *inner.state.balances.entry(*to).or_insert(0) += drip;
        let new_bal = inner.state.get_balance(to);
        Ok(new_bal)
    }
}
