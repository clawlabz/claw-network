//! World state types: agent identity, tokens, reputation, services.

use borsh::{BorshDeserialize, BorshSerialize};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Registered Agent identity on-chain.
#[derive(Debug, Clone, PartialEq, Eq, BorshSerialize, BorshDeserialize, Serialize, Deserialize)]
pub struct AgentIdentity {
    /// Ed25519 public key / address.
    pub address: [u8; 32],
    /// Human-readable name.
    pub name: String,
    /// Arbitrary key-value metadata (e.g., platform associations).
    pub metadata: BTreeMap<String, String>,
    /// Block height at which the agent was registered.
    pub registered_at: u64,
}

/// A custom token definition.
#[derive(Debug, Clone, PartialEq, Eq, BorshSerialize, BorshDeserialize, Serialize, Deserialize)]
pub struct TokenDef {
    /// Token ID: blake3(creator_address || name || nonce).
    pub id: [u8; 32],
    /// Token name.
    pub name: String,
    /// Token symbol (e.g., "MSC").
    pub symbol: String,
    /// Decimal places.
    pub decimals: u8,
    /// Total supply (minted to issuer on creation).
    pub total_supply: u128,
    /// Creator/issuer agent address.
    pub issuer: [u8; 32],
}

/// An on-chain reputation attestation.
#[derive(Debug, Clone, PartialEq, Eq, BorshSerialize, BorshDeserialize, Serialize, Deserialize)]
pub struct ReputationAttestation {
    /// Address of the attester.
    pub from: [u8; 32],
    /// Address of the agent being attested.
    pub to: [u8; 32],
    /// Category (e.g., "game", "task", "service").
    pub category: String,
    /// Score from -100 to +100.
    pub score: i16,
    /// Source platform (arbitrary string, e.g., "clawarena").
    pub platform: String,
    /// Optional memo.
    pub memo: String,
    /// Block height at which this attestation was recorded.
    pub block_height: u64,
}

/// A service entry in the on-chain service registry.
#[derive(Debug, Clone, PartialEq, Eq, BorshSerialize, BorshDeserialize, Serialize, Deserialize)]
pub struct ServiceEntry {
    /// Provider agent address.
    pub provider: [u8; 32],
    /// Service type (e.g., "translation", "code-review").
    pub service_type: String,
    /// Human-readable description.
    pub description: String,
    /// Token accepted for payment (CLW native = all zeros).
    pub price_token: [u8; 32],
    /// Price amount per unit of service.
    pub price_amount: u128,
    /// Endpoint URL for the service.
    pub endpoint: String,
    /// Whether the service is currently active.
    pub active: bool,
}

/// Native CLW token ID (all zeros, represents the native token).
pub const NATIVE_TOKEN_ID: [u8; 32] = [0u8; 32];

/// Gas fee per transaction in native token units.
/// 0.001 CLW = 1_000_000 units (assuming 9 decimals).
pub const GAS_FEE: u128 = 1_000_000;

/// CLW token decimals.
pub const CLW_DECIMALS: u8 = 9;

/// Total CLW supply: 1 billion tokens = 1_000_000_000 * 10^9 base units.
pub const CLW_TOTAL_SUPPLY: u128 = 1_000_000_000_000_000_000;
