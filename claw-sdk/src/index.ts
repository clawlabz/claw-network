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
  AgentRegisterParams,
  TokenTransferParams,
  TokenCreateParams,
  TokenMintTransferParams,
  ReputationAttestParams,
  ServiceRegisterParams,
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
} from './serialization.js';

// Hashing (for advanced usage / testing)
export { transactionHash, transactionHashHex } from './hash.js';
