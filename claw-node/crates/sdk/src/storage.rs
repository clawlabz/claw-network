//! Higher-level typed storage helpers built on top of [`crate::env`].
//!
//! Provides borsh-based serialization and primitive read/write shortcuts.

use borsh::{BorshDeserialize, BorshSerialize};

/// Read a borsh-encoded value from storage.
pub fn get<T: BorshDeserialize>(key: &[u8]) -> Option<T> {
    let bytes = crate::env::storage_get(key)?;
    borsh::from_slice(&bytes).ok()
}

/// Write a borsh-encoded value to storage.
pub fn set<T: BorshSerialize>(key: &[u8], value: &T) {
    let bytes = borsh::to_vec(value).expect("borsh serialize failed");
    crate::env::storage_set(key, &bytes);
}

/// Delete a key from storage.
pub fn remove(key: &[u8]) {
    crate::env::storage_remove(key);
}

/// Check if a key exists.
pub fn exists(key: &[u8]) -> bool {
    crate::env::storage_exists(key)
}

/// Read a `u64` from storage (little-endian).
pub fn get_u64(key: &[u8]) -> Option<u64> {
    let bytes = crate::env::storage_get(key)?;
    if bytes.len() >= 8 {
        Some(u64::from_le_bytes(bytes[..8].try_into().unwrap()))
    } else {
        None
    }
}

/// Write a `u64` to storage (little-endian).
pub fn set_u64(key: &[u8], value: u64) {
    crate::env::storage_set(key, &value.to_le_bytes());
}

/// Read a `u128` from storage (little-endian).
pub fn get_u128(key: &[u8]) -> Option<u128> {
    let bytes = crate::env::storage_get(key)?;
    if bytes.len() >= 16 {
        Some(u128::from_le_bytes(bytes[..16].try_into().unwrap()))
    } else {
        None
    }
}

/// Write a `u128` to storage (little-endian).
pub fn set_u128(key: &[u8], value: u128) {
    crate::env::storage_set(key, &value.to_le_bytes());
}
