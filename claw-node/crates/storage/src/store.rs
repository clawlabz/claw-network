//! redb-backed chain storage.

use borsh::BorshDeserialize;
use claw_types::Block;
use redb::{Database, TableDefinition};
use std::path::Path;
use thiserror::Error;

// Table definitions
const BLOCKS: TableDefinition<u64, &[u8]> = TableDefinition::new("blocks");
const META: TableDefinition<&str, &[u8]> = TableDefinition::new("meta");
const TX_INDEX: TableDefinition<&[u8], u64> = TableDefinition::new("tx_index");

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
        }
        write_txn.commit()?;

        Ok(Self { db })
    }

    /// Store a block and update latest height + tx index.
    pub fn put_block(&self, block: &Block) -> Result<(), StoreError> {
        let block_bytes =
            borsh::to_vec(block).map_err(|e| StoreError::Serialize(e.to_string()))?;

        let write_txn = self.db.begin_write()?;
        {
            let mut blocks = write_txn.open_table(BLOCKS)?;
            blocks.insert(block.height, block_bytes.as_slice())?;

            // Update tx index
            let mut tx_index = write_txn.open_table(TX_INDEX)?;
            for tx in &block.transactions {
                let tx_hash = tx.hash();
                tx_index.insert(tx_hash.as_slice(), block.height)?;
            }

            // Update latest height
            let mut meta = write_txn.open_table(META)?;
            meta.insert(META_LATEST_HEIGHT, block.height.to_le_bytes().as_slice())?;
        }
        write_txn.commit()?;

        Ok(())
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
}
