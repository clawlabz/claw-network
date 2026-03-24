//! Block structure.

use borsh::{BorshDeserialize, BorshSerialize};
use serde::{Deserialize, Serialize};

use crate::transaction::Transaction;

/// A structured event emitted during block production (rewards, fees, burns, contract events).
/// Modeled after Cosmos/Polkadot event patterns for Explorer queryability.
#[derive(Debug, Clone, PartialEq, Eq, BorshSerialize, BorshDeserialize, Serialize, Deserialize)]
pub enum BlockEvent {
    /// A reward or fee was distributed to a recipient (or burned).
    RewardDistributed {
        /// Recipient address (all zeros for burns).
        recipient: [u8; 32],
        /// Amount in base units (9 decimals).
        amount: u128,
        /// Type of reward: "block_reward", "proposer_fee", "ecosystem_fee", "fee_burn".
        reward_type: String,
    },
    /// A structured event emitted by a smart contract via `emit_event`.
    ContractEvent {
        /// Address of the contract that emitted the event.
        contract: [u8; 32],
        /// Index of the transaction within the block that triggered this event.
        tx_index: u32,
        /// Event topic / name (non-empty UTF-8 string).
        topic: String,
        /// Raw event payload bytes.
        data: Vec<u8>,
    },
}

/// A block in the ClawNetwork chain.
///
/// Note: Custom `BorshDeserialize` is implemented to handle backward compatibility
/// with blocks stored before the `signatures` and `events` fields were added.
/// Missing trailing fields default to empty vecs.
#[derive(Debug, Clone, PartialEq, Eq, BorshSerialize, Serialize, Deserialize)]
pub struct Block {
    /// Block height (0 = genesis).
    pub height: u64,
    /// Hash of the previous block (all zeros for genesis).
    pub prev_hash: [u8; 32],
    /// Unix timestamp in seconds.
    pub timestamp: u64,
    /// Address of the validator who produced this block.
    pub validator: [u8; 32],
    /// Transactions included in this block.
    pub transactions: Vec<Transaction>,
    /// Merkle root of the world state after applying this block.
    pub state_root: [u8; 32],
    /// Hash of this block (blake3 of header fields, excluding this field).
    pub hash: [u8; 32],
    /// Validator signatures for BFT finality (address, signature pairs).
    /// Signatures are over the block hash and are appended after hash computation.
    #[serde(default, with = "serde_signatures")]
    pub signatures: Vec<([u8; 32], [u8; 64])>,
    /// Events emitted during block production (rewards, fees, burns).
    /// Not included in block hash computation (appended after consensus).
    #[serde(default)]
    pub events: Vec<BlockEvent>,
}

impl BorshDeserialize for Block {
    fn deserialize_reader<R: std::io::Read>(reader: &mut R) -> std::io::Result<Self> {
        let height = u64::deserialize_reader(reader)?;
        let prev_hash = <[u8; 32]>::deserialize_reader(reader)?;
        let timestamp = u64::deserialize_reader(reader)?;
        let validator = <[u8; 32]>::deserialize_reader(reader)?;
        let transactions = Vec::<Transaction>::deserialize_reader(reader)?;
        let state_root = <[u8; 32]>::deserialize_reader(reader)?;
        let hash = <[u8; 32]>::deserialize_reader(reader)?;

        // Backward-compatible: try to read optional trailing fields.
        // If deserialization fails (old format without these fields), default to empty.
        let signatures = Vec::<([u8; 32], [u8; 64])>::deserialize_reader(reader)
            .unwrap_or_default();
        let events = Vec::<BlockEvent>::deserialize_reader(reader)
            .unwrap_or_default();

        Ok(Block {
            height,
            prev_hash,
            timestamp,
            validator,
            transactions,
            state_root,
            hash,
            signatures,
            events,
        })
    }
}

impl Block {
    /// Compute the block hash from header fields.
    pub fn compute_hash(&self) -> [u8; 32] {
        let mut buf = Vec::new();
        buf.extend_from_slice(&self.height.to_le_bytes());
        buf.extend_from_slice(&self.prev_hash);
        buf.extend_from_slice(&self.timestamp.to_le_bytes());
        buf.extend_from_slice(&self.validator);
        // Hash of all transaction hashes
        for tx in &self.transactions {
            buf.extend_from_slice(&tx.hash());
        }
        buf.extend_from_slice(&self.state_root);
        *blake3::hash(&buf).as_bytes()
    }

    /// Verify that the stored hash matches the computed hash.
    pub fn verify_hash(&self) -> bool {
        self.hash == self.compute_hash()
    }
}

/// Serde helper for `Vec<([u8; 32], [u8; 64])>` since serde does not natively
/// support `[u8; 64]` arrays. Serializes each pair as hex strings.
mod serde_signatures {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    #[derive(Serialize, Deserialize)]
    struct SigPair {
        address: String,
        signature: String,
    }

    pub fn serialize<S>(
        sigs: &Vec<([u8; 32], [u8; 64])>,
        serializer: S,
    ) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let pairs: Vec<SigPair> = sigs
            .iter()
            .map(|(addr, sig)| SigPair {
                address: hex::encode(addr),
                signature: hex::encode(sig),
            })
            .collect();
        pairs.serialize(serializer)
    }

    pub fn deserialize<'de, D>(
        deserializer: D,
    ) -> Result<Vec<([u8; 32], [u8; 64])>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let pairs: Vec<SigPair> = Vec::deserialize(deserializer)?;
        pairs
            .into_iter()
            .map(|p| {
                let addr_bytes: [u8; 32] = hex::decode(&p.address)
                    .map_err(serde::de::Error::custom)?
                    .try_into()
                    .map_err(|_| serde::de::Error::custom("address must be 32 bytes"))?;
                let sig_bytes: [u8; 64] = hex::decode(&p.signature)
                    .map_err(serde::de::Error::custom)?
                    .try_into()
                    .map_err(|_| serde::de::Error::custom("signature must be 64 bytes"))?;
                Ok((addr_bytes, sig_bytes))
            })
            .collect()
    }
}
