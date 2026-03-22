//! redb-backed chain storage.

use borsh::BorshDeserialize;
use claw_types::Block;
use claw_types::transaction::{TxType, TokenTransferPayload, TokenMintTransferPayload, ReputationAttestPayload, ContractCallPayload};
use redb::{Database, ReadableTable, TableDefinition};
use std::path::Path;
use thiserror::Error;

// Table definitions
const BLOCKS: TableDefinition<u64, &[u8]> = TableDefinition::new("blocks");
const META: TableDefinition<&str, &[u8]> = TableDefinition::new("meta");
const TX_INDEX: TableDefinition<&[u8], u64> = TableDefinition::new("tx_index");
/// Maps address (32 bytes hex) → borsh-encoded Vec<(block_height, tx_index_in_block)>.
const ADDRESS_TX_INDEX: TableDefinition<&[u8], &[u8]> = TableDefinition::new("address_tx_index");

const META_LATEST_HEIGHT: &str = "latest_height";
const META_STATE_SNAPSHOT: &str = "state_snapshot";

#[derive(Debug, Error)]
pub enum StoreError {
    #[error("database error: {0}")]
    Db(String),
    #[error("block not found at height {0}")]
    BlockNotFound(u64),
    #[error("serialization error: {0}")]
    Serialize(String),
}

impl From<redb::Error> for StoreError {
    fn from(e: redb::Error) -> Self {
        StoreError::Db(e.to_string())
    }
}

impl From<redb::DatabaseError> for StoreError {
    fn from(e: redb::DatabaseError) -> Self {
        StoreError::Db(e.to_string())
    }
}

impl From<redb::TableError> for StoreError {
    fn from(e: redb::TableError) -> Self {
        StoreError::Db(e.to_string())
    }
}

impl From<redb::TransactionError> for StoreError {
    fn from(e: redb::TransactionError) -> Self {
        StoreError::Db(e.to_string())
    }
}

impl From<redb::StorageError> for StoreError {
    fn from(e: redb::StorageError) -> Self {
        StoreError::Db(e.to_string())
    }
}

impl From<redb::CommitError> for StoreError {
    fn from(e: redb::CommitError) -> Self {
        StoreError::Db(e.to_string())
    }
}

/// Persistent chain storage backed by redb.
pub struct ChainStore {
    db: Database,
}

impl ChainStore {
    /// Open or create a chain store at the given path.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, StoreError> {
        let db = Database::create(path.as_ref()).map_err(|e| StoreError::Db(e.to_string()))?;

        // Ensure tables exist
        let write_txn = db.begin_write()?;
        {
            let _ = write_txn.open_table(BLOCKS)?;
            let _ = write_txn.open_table(META)?;
            let _ = write_txn.open_table(TX_INDEX)?;
            let _ = write_txn.open_table(ADDRESS_TX_INDEX)?;
        }
        write_txn.commit()?;

        Ok(Self { db })
    }

    /// Store a block and update latest height + tx index + address tx index.
    pub fn put_block(&self, block: &Block) -> Result<(), StoreError> {
        let block_bytes =
            borsh::to_vec(block).map_err(|e| StoreError::Serialize(e.to_string()))?;

        let write_txn = self.db.begin_write()?;
        Self::write_block_tables(&write_txn, block, &block_bytes)?;
        write_txn.commit()?;

        Ok(())
    }

    /// Atomically store a block AND state snapshot in a single transaction.
    /// Prevents inconsistency where a block is persisted but the snapshot is not
    /// (or vice versa) due to a crash between two separate writes.
    pub fn put_block_and_snapshot(&self, block: &Block, snapshot: &[u8]) -> Result<(), StoreError> {
        let block_bytes =
            borsh::to_vec(block).map_err(|e| StoreError::Serialize(e.to_string()))?;

        let write_txn = self.db.begin_write()?;
        Self::write_block_tables(&write_txn, block, &block_bytes)?;
        {
            let mut meta = write_txn.open_table(META)?;
            meta.insert(META_STATE_SNAPSHOT, snapshot)?;
        }
        write_txn.commit()?;

        Ok(())
    }

    /// Internal helper: write block data, tx indexes, and latest height into an
    /// existing write transaction. Shared by `put_block` and `put_block_and_snapshot`.
    fn write_block_tables(
        write_txn: &redb::WriteTransaction,
        block: &Block,
        block_bytes: &[u8],
    ) -> Result<(), StoreError> {
        let mut blocks = write_txn.open_table(BLOCKS)?;
        blocks.insert(block.height, block_bytes)?;

        // Update tx index + address tx index
        let mut tx_index = write_txn.open_table(TX_INDEX)?;
        let mut addr_tx_index = write_txn.open_table(ADDRESS_TX_INDEX)?;

        for (tx_idx, tx) in block.transactions.iter().enumerate() {
            let tx_hash = tx.hash();
            tx_index.insert(tx_hash.as_slice(), block.height)?;

            let entry = (block.height, tx_idx as u32);

            // Index the sender (from)
            Self::append_address_entry(&mut addr_tx_index, &tx.from, entry)?;

            // Index the recipient (to) if one exists in the payload
            if let Some(to_addr) = Self::extract_to_address(tx.tx_type, &tx.payload) {
                if to_addr != tx.from {
                    Self::append_address_entry(&mut addr_tx_index, &to_addr, entry)?;
                }
            }
        }

        // Update latest height
        let mut meta = write_txn.open_table(META)?;
        meta.insert(META_LATEST_HEIGHT, block.height.to_le_bytes().as_slice())?;

        Ok(())
    }

    /// Append a (block_height, tx_index) entry to the address tx index.
    fn append_address_entry(
        table: &mut redb::Table<&[u8], &[u8]>,
        address: &[u8; 32],
        entry: (u64, u32),
    ) -> Result<(), StoreError> {
        let mut entries: Vec<(u64, u32)> = match table.get(address.as_slice())? {
            Some(data) => {
                borsh::from_slice::<Vec<(u64, u32)>>(data.value()).unwrap_or_default()
            }
            None => Vec::new(),
        };
        entries.push(entry);
        let encoded = borsh::to_vec(&entries).map_err(|e| StoreError::Serialize(e.to_string()))?;
        table.insert(address.as_slice(), encoded.as_slice())?;
        Ok(())
    }

    /// Extract the `to` address from a transaction payload based on tx_type.
    fn extract_to_address(tx_type: TxType, payload: &[u8]) -> Option<[u8; 32]> {
        match tx_type {
            TxType::TokenTransfer => {
                borsh::from_slice::<TokenTransferPayload>(payload)
                    .ok()
                    .map(|p| p.to)
            }
            TxType::TokenMintTransfer => {
                borsh::from_slice::<TokenMintTransferPayload>(payload)
                    .ok()
                    .map(|p| p.to)
            }
            TxType::ReputationAttest => {
                borsh::from_slice::<ReputationAttestPayload>(payload)
                    .ok()
                    .map(|p| p.to)
            }
            TxType::ContractDeploy => None,
            TxType::ContractCall => {
                borsh::from_slice::<ContractCallPayload>(payload)
                    .ok()
                    .map(|p| p.contract)
            }
            TxType::AgentRegister | TxType::TokenCreate | TxType::ServiceRegister
            | TxType::StakeDeposit | TxType::StakeWithdraw | TxType::StakeClaim
            | TxType::PlatformActivityReport | TxType::TokenApprove | TxType::TokenBurn
            | TxType::ChangeDelegation => None,
        }
    }

    /// Get a block by height.
    pub fn get_block(&self, height: u64) -> Result<Option<Block>, StoreError> {
        let read_txn = self.db.begin_read()?;
        let table = read_txn.open_table(BLOCKS)?;

        match table.get(height)? {
            Some(data) => {
                let block = Block::try_from_slice(data.value())
                    .map_err(|e| StoreError::Serialize(e.to_string()))?;
                Ok(Some(block))
            }
            None => Ok(None),
        }
    }

    /// Get the latest block height, or None if no blocks stored.
    pub fn get_latest_height(&self) -> Result<Option<u64>, StoreError> {
        let read_txn = self.db.begin_read()?;
        let table = read_txn.open_table(META)?;

        match table.get(META_LATEST_HEIGHT)? {
            Some(data) => {
                let bytes: [u8; 8] = data.value().try_into().unwrap_or([0u8; 8]);
                Ok(Some(u64::from_le_bytes(bytes)))
            }
            None => Ok(None),
        }
    }

    /// Get the block height containing a given transaction hash.
    pub fn get_tx_block_height(&self, tx_hash: &[u8; 32]) -> Result<Option<u64>, StoreError> {
        let read_txn = self.db.begin_read()?;
        let table = read_txn.open_table(TX_INDEX)?;

        match table.get(tx_hash.as_slice())? {
            Some(data) => Ok(Some(data.value())),
            None => Ok(None),
        }
    }

    /// Get transactions for an address, sorted by block height descending (newest first).
    /// Returns a Vec of (block_height, tx_index, Transaction, block_timestamp).
    pub fn get_transactions_by_address(
        &self,
        address: &[u8; 32],
        limit: usize,
        offset: usize,
    ) -> Result<Vec<(u64, u32, claw_types::Transaction, u64)>, StoreError> {
        let read_txn = self.db.begin_read()?;
        let addr_table = read_txn.open_table(ADDRESS_TX_INDEX)?;

        let entries: Vec<(u64, u32)> = match addr_table.get(address.as_slice())? {
            Some(data) => borsh::from_slice(data.value()).unwrap_or_default(),
            None => return Ok(Vec::new()),
        };

        // Sort by block_height descending, then tx_index descending
        let mut sorted = entries;
        sorted.sort_by(|a, b| b.0.cmp(&a.0).then(b.1.cmp(&a.1)));

        // Apply pagination
        let paginated: Vec<(u64, u32)> = sorted
            .into_iter()
            .skip(offset)
            .take(limit)
            .collect();

        let blocks_table = read_txn.open_table(BLOCKS)?;
        let mut results = Vec::with_capacity(paginated.len());

        for (height, tx_idx) in paginated {
            if let Some(block_data) = blocks_table.get(height)? {
                if let Ok(block) = Block::try_from_slice(block_data.value()) {
                    if let Some(tx) = block.transactions.get(tx_idx as usize) {
                        results.push((height, tx_idx, tx.clone(), block.timestamp));
                    }
                }
            }
        }

        Ok(results)
    }

    /// Store a state snapshot (borsh-serialized WorldState).
    pub fn put_state_snapshot(&self, data: &[u8]) -> Result<(), StoreError> {
        let write_txn = self.db.begin_write()?;
        {
            let mut meta = write_txn.open_table(META)?;
            meta.insert(META_STATE_SNAPSHOT, data)?;
        }
        write_txn.commit()?;
        Ok(())
    }

    /// Get the latest state snapshot.
    pub fn get_state_snapshot(&self) -> Result<Option<Vec<u8>>, StoreError> {
        let read_txn = self.db.begin_read()?;
        let table = read_txn.open_table(META)?;

        match table.get(META_STATE_SNAPSHOT)? {
            Some(data) => Ok(Some(data.value().to_vec())),
            None => Ok(None),
        }
    }

    /// Prune (delete) all blocks with height in [1, below_height).
    /// Genesis block (height 0) is always preserved.
    /// Returns the number of blocks removed.
    pub fn prune_blocks_below(&self, below_height: u64) -> u64 {
        if below_height <= 1 {
            return 0;
        }
        let write_txn = match self.db.begin_write() {
            Ok(txn) => txn,
            Err(e) => {
                tracing::error!(error = %e, "Failed to begin write txn for pruning");
                return 0;
            }
        };
        let mut count = 0u64;
        {
            let mut table = match write_txn.open_table(BLOCKS) {
                Ok(t) => t,
                Err(e) => {
                    tracing::error!(error = %e, "Failed to open blocks table for pruning");
                    return 0;
                }
            };
            // Collect keys to remove: range [1, below_height)
            let keys_to_remove: Vec<u64> = match table.range(1..below_height) {
                Ok(iter) => iter
                    .filter_map(|entry| entry.ok().map(|(k, _)| k.value()))
                    .collect(),
                Err(e) => {
                    tracing::error!(error = %e, "Failed to iterate blocks for pruning");
                    return 0;
                }
            };
            for key in keys_to_remove {
                if table.remove(key).is_ok() {
                    count += 1;
                }
            }
        }
        if let Err(e) = write_txn.commit() {
            tracing::error!(error = %e, "Failed to commit pruning transaction");
            return 0;
        }
        count
    }
}
