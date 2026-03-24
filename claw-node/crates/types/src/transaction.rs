//! Transaction types and payloads.

use borsh::{BorshDeserialize, BorshSerialize};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

mod serde_sig {
    use serde::{self, Deserialize, Deserializer, Serializer};

    pub fn serialize<S>(bytes: &[u8; 64], serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let hex_str = bytes.iter().map(|b| format!("{b:02x}")).collect::<String>();
        serializer.serialize_str(&hex_str)
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<[u8; 64], D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        let bytes: Vec<u8> = (0..s.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&s[i..i + 2], 16).map_err(serde::de::Error::custom))
            .collect::<Result<Vec<u8>, _>>()?;
        let arr: [u8; 64] = bytes
            .try_into()
            .map_err(|_| serde::de::Error::custom("expected 64 bytes"))?;
        Ok(arr)
    }
}

/// The native transaction types supported by ClawNetwork.
#[derive(Debug, Clone, Copy, PartialEq, Eq, BorshSerialize, BorshDeserialize, Serialize, Deserialize)]
#[borsh(use_discriminant = true)]
#[repr(u8)]
pub enum TxType {
    AgentRegister = 0,
    TokenTransfer = 1,
    TokenCreate = 2,
    TokenMintTransfer = 3,
    ReputationAttest = 4,
    ServiceRegister = 5,
    ContractDeploy = 6,
    ContractCall = 7,
    StakeDeposit = 8,
    StakeWithdraw = 9,
    StakeClaim = 10,
    PlatformActivityReport = 11,
    TokenApprove = 12,
    TokenBurn = 13,
    ChangeDelegation = 14,
    MinerRegister = 15,
    MinerHeartbeat = 16,
    /// Announce intent to upgrade a contract (starts the timelock).
    ContractUpgradeAnnounce = 17,
    /// Execute a previously announced contract upgrade (after delay has elapsed).
    ContractUpgradeExecute = 18,
}

/// A signed transaction on ClawNetwork.
#[derive(Debug, Clone, PartialEq, Eq, BorshSerialize, BorshDeserialize, Serialize, Deserialize)]
pub struct Transaction {
    /// Transaction type discriminator.
    pub tx_type: TxType,
    /// Sender address (Ed25519 public key).
    pub from: [u8; 32],
    /// Nonce for replay protection (must equal sender's current nonce + 1).
    pub nonce: u64,
    /// Type-specific payload (borsh-encoded).
    pub payload: Vec<u8>,
    /// Ed25519 signature over (tx_type || from || nonce || payload).
    #[serde(with = "serde_sig")]
    pub signature: [u8; 64],
}

// --- Payload types ---

#[derive(Debug, Clone, PartialEq, Eq, BorshSerialize, BorshDeserialize, Serialize, Deserialize)]
pub struct AgentRegisterPayload {
    pub name: String,
    pub metadata: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq, BorshSerialize, BorshDeserialize, Serialize, Deserialize)]
pub struct TokenTransferPayload {
    pub to: [u8; 32],
    pub amount: u128,
}

#[derive(Debug, Clone, PartialEq, Eq, BorshSerialize, BorshDeserialize, Serialize, Deserialize)]
pub struct TokenCreatePayload {
    pub name: String,
    pub symbol: String,
    pub decimals: u8,
    pub total_supply: u128,
}

#[derive(Debug, Clone, PartialEq, Eq, BorshSerialize, BorshDeserialize, Serialize, Deserialize)]
pub struct TokenMintTransferPayload {
    pub token_id: [u8; 32],
    pub to: [u8; 32],
    pub amount: u128,
}

#[derive(Debug, Clone, PartialEq, Eq, BorshSerialize, BorshDeserialize, Serialize, Deserialize)]
pub struct ReputationAttestPayload {
    pub to: [u8; 32],
    pub category: String,
    pub score: i16,
    pub platform: String,
    pub memo: String,
}

#[derive(Debug, Clone, PartialEq, Eq, BorshSerialize, BorshDeserialize, Serialize, Deserialize)]
pub struct ServiceRegisterPayload {
    pub service_type: String,
    pub description: String,
    pub price_token: [u8; 32],
    pub price_amount: u128,
    pub endpoint: String,
    pub active: bool,
}

/// Payload for deploying a new smart contract.
#[derive(Debug, Clone, PartialEq, Eq, BorshSerialize, BorshDeserialize, Serialize, Deserialize)]
pub struct ContractDeployPayload {
    /// Wasm bytecode of the contract.
    pub code: Vec<u8>,
    /// Optional constructor method name (empty string = no constructor).
    pub init_method: String,
    /// Optional constructor arguments (borsh-encoded).
    pub init_args: Vec<u8>,
}

/// Payload for calling a deployed smart contract.
#[derive(Debug, Clone, PartialEq, Eq, BorshSerialize, BorshDeserialize, Serialize, Deserialize)]
pub struct ContractCallPayload {
    /// Address of the deployed contract.
    pub contract: [u8; 32],
    /// Method name to invoke.
    pub method: String,
    /// Method arguments (borsh-encoded).
    pub args: Vec<u8>,
    /// Native CLAW value to send with the call.
    pub value: u128,
}

/// Payload for depositing stake to become a validator.
#[derive(Debug, Clone, PartialEq, Eq, BorshSerialize, BorshDeserialize, Serialize, Deserialize)]
pub struct StakeDepositPayload {
    /// Amount of CLAW to stake (in base units, 9 decimals).
    pub amount: u128,
    /// Optional: delegate block production to this address.
    /// If all-zeros, the staker is also the validator (self-stake).
    pub validator: [u8; 32],
    /// Commission rate in basis points (0-10000). The validator keeps this
    /// percentage of block rewards; the delegator gets the rest.
    /// Default: 10000 (validator keeps all, backward compatible with self-stake).
    pub commission_bps: u16,
}

/// Payload for initiating a stake withdrawal (unbonding).
#[derive(Debug, Clone, PartialEq, Eq, BorshSerialize, BorshDeserialize, Serialize, Deserialize)]
pub struct StakeWithdrawPayload {
    /// Amount of CLAW to unbond (in base units, 9 decimals).
    pub amount: u128,
    /// Validator to unstake from. All-zeros = self (backward compat).
    pub validator: [u8; 32],
}

/// Payload for claiming unbonded stake (no fields needed — claims all mature entries).
#[derive(Debug, Clone, PartialEq, Eq, BorshSerialize, BorshDeserialize, Serialize, Deserialize)]
pub struct StakeClaimPayload;

/// Payload for approving a spender to transfer custom tokens on behalf of the owner.
#[derive(Debug, Clone, PartialEq, Eq, BorshSerialize, BorshDeserialize, Serialize, Deserialize)]
pub struct TokenApprovePayload {
    /// The custom token ID.
    pub token_id: [u8; 32],
    /// The address being approved to spend tokens.
    pub spender: [u8; 32],
    /// The approved amount. Setting to 0 revokes the approval.
    pub amount: u128,
}

/// Payload for burning (destroying) custom tokens from the sender's balance.
#[derive(Debug, Clone, PartialEq, Eq, BorshSerialize, BorshDeserialize, Serialize, Deserialize)]
pub struct TokenBurnPayload {
    /// The custom token ID.
    pub token_id: [u8; 32],
    /// The amount to burn.
    pub amount: u128,
}

/// Payload for changing delegation of an existing validator stake.
///
/// Only the current delegator (or the validator itself for self-stake) can
/// transfer delegation to a new owner. The stake amount stays unchanged.
#[derive(Debug, Clone, PartialEq, Eq, BorshSerialize, BorshDeserialize, Serialize, Deserialize)]
pub struct ChangeDelegationPayload {
    /// The validator address whose delegation to change.
    pub validator: [u8; 32],
    /// The new delegator/owner address.
    pub new_owner: [u8; 32],
    /// New commission rate in basis points (0-10000).
    pub commission_bps: u16,
}

/// A single activity entry within a PlatformActivityReport.
#[derive(Debug, Clone, PartialEq, Eq, BorshSerialize, BorshDeserialize, Serialize, Deserialize)]
pub struct ActivityEntry {
    /// Address of the agent whose activity is being reported.
    pub agent: [u8; 32],
    /// Number of actions performed in this reporting period.
    pub action_count: u32,
    /// Type of action (e.g., "game_played", "task_completed", "query_served").
    pub action_type: String,
}

/// Payload for submitting platform activity reports (tx type 11).
///
/// Only Platform Agents (staked >= 50,000 CLAW) can submit these reports.
/// Each Platform Agent can submit at most once per epoch (100 blocks).
#[derive(Debug, Clone, PartialEq, Eq, BorshSerialize, BorshDeserialize, Serialize, Deserialize)]
pub struct PlatformActivityReportPayload {
    /// Activity entries for agents on this platform.
    pub reports: Vec<ActivityEntry>,
}

/// Payload for registering as a miner on ClawNetwork.
#[derive(Debug, Clone, PartialEq, Eq, BorshSerialize, BorshDeserialize, Serialize, Deserialize)]
pub struct MinerRegisterPayload {
    /// Miner tier (maps to MinerTier enum value).
    pub tier: u8,
    /// IP address bytes (4 bytes for IPv4, 16 for IPv6).
    pub ip_addr: Vec<u8>,
    /// Human-readable miner name.
    pub name: String,
}

/// Payload for submitting a miner heartbeat.
#[derive(Debug, Clone, PartialEq, Eq, BorshSerialize, BorshDeserialize, Serialize, Deserialize)]
pub struct MinerHeartbeatPayload {
    /// Hash of the latest block the miner has synced.
    pub latest_block_hash: [u8; 32],
    /// Height of the latest block the miner has synced.
    pub latest_height: u64,
}

/// Payload for announcing a contract upgrade (starts the timelock).
///
/// The caller must be the contract's admin. After `UPGRADE_DELAY_BLOCKS` have
/// elapsed, the admin can submit a `ContractUpgradeExecute` transaction with
/// the actual new Wasm bytecode.
#[derive(Debug, Clone, PartialEq, Eq, BorshSerialize, BorshDeserialize, Serialize, Deserialize)]
pub struct ContractUpgradeAnnouncePayload {
    /// Address of the contract to upgrade.
    pub contract: [u8; 32],
    /// blake3 hash of the new Wasm bytecode that will be submitted on execute.
    pub new_code_hash: [u8; 32],
}

/// Payload for executing a previously announced contract upgrade.
///
/// The caller must be the contract's admin and the timelock delay must have
/// elapsed since the announce. The `new_code` must hash to the same value
/// that was committed in the announce transaction.
#[derive(Debug, Clone, PartialEq, Eq, BorshSerialize, BorshDeserialize, Serialize, Deserialize)]
pub struct ContractUpgradeExecutePayload {
    /// Address of the contract to upgrade.
    pub contract: [u8; 32],
    /// Full Wasm bytecode of the new contract version.
    pub new_code: Vec<u8>,
    /// Optional migration method to call on the new code immediately after upgrade.
    /// `None` means no migration is run.
    pub migrate_method: Option<String>,
    /// Arguments passed to the migration method (borsh-encoded).
    pub migrate_args: Vec<u8>,
}

impl Transaction {
    /// Returns the bytes that are signed (everything except the signature field).
    pub fn signable_bytes(&self) -> Vec<u8> {
        let mut buf = Vec::new();
        buf.push(self.tx_type as u8);
        buf.extend_from_slice(&self.from);
        buf.extend_from_slice(&self.nonce.to_le_bytes());
        buf.extend_from_slice(&self.payload);
        buf
    }

    /// Compute the transaction hash (blake3 of the full serialized tx).
    pub fn hash(&self) -> [u8; 32] {
        let bytes = borsh::to_vec(self).expect("tx serialization cannot fail");
        *blake3::hash(&bytes).as_bytes()
    }
}
