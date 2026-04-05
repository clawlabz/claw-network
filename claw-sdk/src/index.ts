// ---------------------------------------------------------------------------
// @clawlabz/clawnetwork-sdk — Main entry point
// ---------------------------------------------------------------------------

// Core classes
export { Wallet, toHex, fromHex } from './wallet.js';
export { ClawClient } from './client.js';
export { RpcClient, RpcError, DEFAULT_RPC_URL } from './rpc.js';

// Types
export { TxType } from './types.js';
export type {
  Transaction,
  ClawClientConfig,
  WalletLike,
  AgentIdentity,
  TokenDef,
  ReputationAttestation,
  ServiceEntry,
  BlockInfo,
  TransactionReceipt,
  TransactionResponse,
  AgentRegisterParams,
  TokenTransferParams,
  TokenCreateParams,
  TokenMintTransferParams,
  ReputationAttestParams,
  ServiceRegisterParams,
  StakeDepositParams,
  StakeWithdrawParams,
  ChangeDelegationParams,
  ContractDeployParams,
  ContractCallParams,
  MinerRegisterParams,
  MinerHeartbeatParams,
} from './types.js';

// Serialization (for advanced usage / testing)
export {
  signableBytes,
  serializeTransaction,
  encodeAgentRegisterPayload,
  encodeTokenTransferPayload,
  encodeTokenCreatePayload,
  encodeTokenMintTransferPayload,
  encodeReputationAttestPayload,
  encodeServiceRegisterPayload,
  encodeStakeDepositPayload,
  encodeStakeWithdrawPayload,
  encodeStakeClaimPayload,
  encodeChangeDelegationPayload,
  encodeContractDeployPayload,
  encodeContractCallPayload,
  encodeMinerRegisterPayload,
  encodeMinerHeartbeatPayload,
} from './serialization.js';

// Hashing (for advanced usage / testing)
export { transactionHash, transactionHashHex } from './hash.js';
