//! Cryptographic primitives for ClawNetwork.
//!
//! - Ed25519 key generation, signing, verification
//! - Blake3 hashing
//! - Merkle tree for state roots

pub mod keys;
pub mod merkle;
pub mod signer;

// Re-export for downstream crates
pub use ed25519_dalek;
