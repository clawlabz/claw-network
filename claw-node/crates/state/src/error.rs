//! State transition errors.

use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum StateError {
    #[error("invalid signature")]
    InvalidSignature,

    #[error("invalid nonce: expected {expected}, got {got}")]
    InvalidNonce { expected: u64, got: u64 },

    #[error("insufficient balance: need {need}, have {have}")]
    InsufficientBalance { need: u128, have: u128 },

    #[error("agent already registered")]
    AgentAlreadyRegistered,

    #[error("agent not registered")]
    AgentNotRegistered,

    #[error("token already exists")]
    TokenAlreadyExists,

    #[error("token not found")]
    TokenNotFound,

    #[error("cannot attest to self")]
    SelfAttestation,

    #[error("score out of range: {0} (must be -100..=100)")]
    ScoreOutOfRange(i16),

    #[error("amount must be greater than zero")]
    ZeroAmount,

    #[error("name too long: {len} bytes (max {max})")]
    NameTooLong { len: usize, max: usize },

    #[error("cannot use native token ID for custom token transfer")]
    NativeTokenIdForCustom,

    #[error("payload deserialization failed: {0}")]
    PayloadDeserialize(String),

    #[error("total supply must be greater than zero")]
    ZeroSupply,

    #[error("transaction payload too large: {len} bytes (max {max})")]
    PayloadTooLarge { len: usize, max: usize },

    #[error("balance overflow: adding {amount} to {balance} would exceed u128::MAX")]
    BalanceOverflow { amount: u128, balance: u128 },

    #[error("description too long: {len} bytes (max {max})")]
    DescriptionTooLong { len: usize, max: usize },

    #[error("symbol too long: {len} bytes (max {max})")]
    SymbolTooLong { len: usize, max: usize },

    #[error("endpoint too long: {len} bytes (max {max})")]
    EndpointTooLong { len: usize, max: usize },

    #[error("metadata too large: {len} entries (max {max})")]
    MetadataTooLarge { len: usize, max: usize },

    #[error("memo too long: {len} bytes (max {max})")]
    MemoTooLong { len: usize, max: usize },

    #[error("attestation limit reached: maximum {max} attestations per attester-target pair")]
    AttestationLimitReached { max: usize },

    #[error("contract not found: {0}")]
    ContractNotFound(String),

    #[error("contract execution failed: {0}")]
    ContractExecutionFailed(String),

    #[error("contract code too large: {size} bytes (max {max})")]
    ContractCodeTooLarge { size: usize, max: usize },

    #[error("invalid contract method: {0}")]
    InvalidContractMethod(String),

    #[error("contract already exists at address")]
    ContractAlreadyExists,

    #[error("staking error: {0}")]
    StakeError(String),

    #[error("no claimable unbonding entries")]
    NoClaimableUnbonding,

    #[error("platform agent stake too low: need {need}, have {have}")]
    PlatformStakeTooLow { need: u128, have: u128 },

    #[error("platform agent already reported this epoch")]
    PlatformReportAlreadySubmitted,

    #[error("action_type too long: {len} bytes (max {max})")]
    ActionTypeTooLong { len: usize, max: usize },

    #[error("too many activity entries: {len} (max {max})")]
    TooManyActivityEntries { len: usize, max: usize },
}
