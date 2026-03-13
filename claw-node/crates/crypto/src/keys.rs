//! Ed25519 key pair generation and management.

use ed25519_dalek::{SigningKey, VerifyingKey};
use rand::rngs::OsRng;

/// Generate a new Ed25519 keypair.
pub fn generate_keypair() -> (SigningKey, VerifyingKey) {
    let signing_key = SigningKey::generate(&mut OsRng);
    let verifying_key = signing_key.verifying_key();
    (signing_key, verifying_key)
}

/// Derive the 32-byte address from a verifying (public) key.
pub fn address_from_verifying_key(vk: &VerifyingKey) -> [u8; 32] {
    vk.to_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generate_keypair_produces_valid_pair() {
        let (sk, vk) = generate_keypair();
        let addr = address_from_verifying_key(&vk);
        assert_eq!(addr, vk.to_bytes());
        // Signing key can reproduce verifying key
        assert_eq!(sk.verifying_key(), vk);
    }

    #[test]
    fn different_keypairs_have_different_addresses() {
        let (_, vk1) = generate_keypair();
        let (_, vk2) = generate_keypair();
        assert_ne!(
            address_from_verifying_key(&vk1),
            address_from_verifying_key(&vk2)
        );
    }
}
