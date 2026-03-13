#[cfg(test)]
mod tests {
    use crate::ChainStore;
    use claw_types::block::Block;
    use claw_types::transaction::{Transaction, TxType};

    fn temp_store() -> (ChainStore, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let store = ChainStore::open(dir.path().join("test.redb")).unwrap();
        (store, dir)
    }

    fn make_block(height: u64) -> Block {
        Block {
            height,
            prev_hash: [0u8; 32],
            timestamp: 1710000000 + height,
            validator: [1u8; 32],
            transactions: vec![Transaction {
                tx_type: TxType::AgentRegister,
                from: [2u8; 32],
                nonce: height,
                payload: vec![height as u8],
                signature: [3u8; 64],
            }],
            state_root: [4u8; 32],
            hash: [5u8; 32],
        }
    }

    #[test]
    fn put_and_get_block() {
        let (store, _dir) = temp_store();
        let block = make_block(0);
        store.put_block(&block).unwrap();

        let loaded = store.get_block(0).unwrap().unwrap();
        assert_eq!(loaded.height, 0);
        assert_eq!(loaded.transactions.len(), 1);
    }

    #[test]
    fn get_nonexistent_block_returns_none() {
        let (store, _dir) = temp_store();
        assert!(store.get_block(999).unwrap().is_none());
    }

    #[test]
    fn latest_height_updates() {
        let (store, _dir) = temp_store();
        assert!(store.get_latest_height().unwrap().is_none());

        store.put_block(&make_block(0)).unwrap();
        assert_eq!(store.get_latest_height().unwrap(), Some(0));

        store.put_block(&make_block(1)).unwrap();
        assert_eq!(store.get_latest_height().unwrap(), Some(1));
    }

    #[test]
    fn tx_index_lookup() {
        let (store, _dir) = temp_store();
        let block = make_block(42);
        let tx_hash = block.transactions[0].hash();
        store.put_block(&block).unwrap();

        assert_eq!(store.get_tx_block_height(&tx_hash).unwrap(), Some(42));
    }

    #[test]
    fn state_snapshot_roundtrip() {
        let (store, _dir) = temp_store();
        assert!(store.get_state_snapshot().unwrap().is_none());

        let data = b"test snapshot data";
        store.put_state_snapshot(data).unwrap();

        let loaded = store.get_state_snapshot().unwrap().unwrap();
        assert_eq!(loaded, data);
    }

    #[test]
    fn multiple_blocks() {
        let (store, _dir) = temp_store();
        for i in 0..10 {
            store.put_block(&make_block(i)).unwrap();
        }
        assert_eq!(store.get_latest_height().unwrap(), Some(9));

        for i in 0..10 {
            let block = store.get_block(i).unwrap().unwrap();
            assert_eq!(block.height, i);
        }
    }
}
