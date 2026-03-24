use borsh::BorshSerialize;
use ed25519_dalek::{Signer, SigningKey, VerifyingKey};

/// Mirror of the node's TxType enum (only the values needed by this CLI).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum TxType {
    ContractDeploy = 6,
    ContractCall = 7,
}

/// A signed transaction ready for JSON-RPC submission.
#[derive(Debug, Clone)]
pub struct SignedTransaction {
    pub tx_type: TxType,
    pub from: [u8; 32],
    pub nonce: u64,
    pub payload: Vec<u8>,
    pub signature: [u8; 64],
}

// --- Internal borsh-serializable structs that mirror the node's payload types ---

#[derive(BorshSerialize)]
struct ContractDeployPayloadBorsh<'a> {
    code: &'a [u8],
    init_method: String,
    init_args: &'a [u8],
}

#[derive(BorshSerialize)]
struct ContractCallPayloadBorsh<'a> {
    contract: [u8; 32],
    method: String,
    args: &'a [u8],
    value: u128,
}

/// Borsh-encode a ContractDeployPayload matching the node's struct layout.
pub fn encode_contract_deploy_payload(
    code: &[u8],
    init_method: &str,
    init_args: &[u8],
) -> Vec<u8> {
    let payload = ContractDeployPayloadBorsh {
        code,
        init_method: init_method.to_string(),
        init_args,
    };
    borsh::to_vec(&payload).expect("ContractDeployPayload serialization cannot fail")
}

/// Borsh-encode a ContractCallPayload matching the node's struct layout.
pub fn encode_contract_call_payload(
    contract: &[u8; 32],
    method: &str,
    args: &[u8],
    value: u128,
) -> Vec<u8> {
    let payload = ContractCallPayloadBorsh {
        contract: *contract,
        method: method.to_string(),
        args,
        value,
    };
    borsh::to_vec(&payload).expect("ContractCallPayload serialization cannot fail")
}

/// Derive the contract address from deployer pubkey and deploy nonce.
///
/// Matches VmEngine::derive_contract_address exactly:
///   blake3("claw_contract_v1:" || deployer_bytes || nonce_le_bytes)
pub fn derive_contract_address(deployer: &[u8; 32], nonce: u64) -> [u8; 32] {
    let mut buf = Vec::with_capacity(17 + 32 + 8);
    buf.extend_from_slice(b"claw_contract_v1:");
    buf.extend_from_slice(deployer);
    buf.extend_from_slice(&nonce.to_le_bytes());
    *blake3::hash(&buf).as_bytes()
}

/// Build and sign a transaction.
///
/// The signable bytes follow the node's Transaction::signable_bytes():
///   tx_type_u8 || from_32 || nonce_le_8 || payload
pub fn build_and_sign_transaction(
    tx_type: TxType,
    signing_key: &SigningKey,
    nonce: u64,
    payload: Vec<u8>,
) -> SignedTransaction {
    let verifying_key = VerifyingKey::from(signing_key);
    let from = verifying_key.to_bytes();

    // Build signable bytes exactly as the node does
    let mut signable = Vec::new();
    signable.push(tx_type as u8);
    signable.extend_from_slice(&from);
    signable.extend_from_slice(&nonce.to_le_bytes());
    signable.extend_from_slice(&payload);

    let sig = signing_key.sign(&signable);

    SignedTransaction {
        tx_type,
        from,
        nonce,
        payload,
        signature: sig.to_bytes(),
    }
}
