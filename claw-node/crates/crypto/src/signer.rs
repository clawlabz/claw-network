//! Transaction signing and verification.

use claw_types::transaction::Transaction;
use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum SignError {
    #[error("signature verification failed")]
    InvalidSignature,
    #[error("public key does not match transaction sender")]
    SenderMismatch,
}

/// Sign a transaction. Sets the `from` field to the signer's address
/// and fills in the `signature` field.
pub fn sign_transaction(tx: &mut Transaction, signing_key: &SigningKey) {
    tx.from = signing_key.verifying_key().to_bytes();
    let msg = tx.signable_bytes();
    let sig = signing_key.sign(&msg);
    tx.signature = sig.to_bytes();
}

/// Verify the signature on a transaction.
pub fn verify_transaction(tx: &Transaction) -> Result<(), SignError> {
    let vk = VerifyingKey::from_bytes(&tx.from).map_err(|_| SignError::InvalidSignature)?;
    let sig = Signature::from_bytes(&tx.signature);
    let msg = tx.signable_bytes();
    vk.verify_strict(&msg, &sig)
        .map_err(|_| SignError::InvalidSignature)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keys::generate_keypair;
    use claw_types::transaction::TxType;

    fn make_test_tx() -> Transaction {
        Transaction {
            tx_type: TxType::TokenTransfer,
            from: [0u8; 32],
            nonce: 1,
            payload: vec![1, 2, 3],
            signature: [0u8; 64],
        }
    }

    #[test]
    fn sign_and_verify_roundtrip() {
        let (sk, _) = generate_keypair();
        let mut tx = make_test_tx();
        sign_transaction(&mut tx, &sk);
        assert!(verify_transaction(&tx).is_ok());
    }

    #[test]
    fn tampered_payload_fails_verification() {
        let (sk, _) = generate_keypair();
        let mut tx = make_test_tx();
        sign_transaction(&mut tx, &sk);
        tx.payload.push(99); // tamper
        assert!(verify_transaction(&tx).is_err());
    }

    #[test]
    fn tampered_nonce_fails_verification() {
        let (sk, _) = generate_keypair();
        let mut tx = make_test_tx();
        sign_transaction(&mut tx, &sk);
        tx.nonce = 999; // tamper
        assert!(verify_transaction(&tx).is_err());
    }

    #[test]
    fn wrong_sender_fails_verification() {
        let (sk, _) = generate_keypair();
        let mut tx = make_test_tx();
        sign_transaction(&mut tx, &sk);
        tx.from = [1u8; 32]; // wrong sender
        assert!(verify_transaction(&tx).is_err());
    }
}
