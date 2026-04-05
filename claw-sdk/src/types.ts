// ---------------------------------------------------------------------------
// ClawNetwork SDK — Type definitions
// ---------------------------------------------------------------------------

/** Transaction type discriminator (matches Rust TxType enum). */
export enum TxType {
  AgentRegister = 0,
  TokenTransfer = 1,
  TokenCreate = 2,
  TokenMintTransfer = 3,
  /** @deprecated Use Agent Score system instead */
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
  ContractUpgradeAnnounce = 17,
  ContractUpgradeExecute = 18,
}

/** @deprecated Use TokenTransfer instead (same value, renamed for clarity) */
export const Transfer = TxType.TokenTransfer;

/** A signed transaction on ClawNetwork. */
export interface Transaction {
  txType: TxType;
  from: Uint8Array; // 32 bytes
  nonce: bigint; // u64
  payload: Uint8Array; // borsh-encoded payload
  signature: Uint8Array; // 64 bytes
}

// --- Payload parameter types (user-facing) ---

export interface AgentRegisterParams {
  name: string;
  metadata: Record<string, string>;
}

export interface TokenTransferParams {
  to: string; // hex address
  amount: bigint; // u128
}

export interface TokenCreateParams {
  name: string;
  symbol: string;
  decimals: number; // u8
  totalSupply: bigint; // u128
}

export interface TokenMintTransferParams {
  tokenId: string; // hex
  to: string; // hex
  amount: bigint; // u128
}

export interface ReputationAttestParams {
  to: string; // hex
  category: string;
  score: number; // i16
  platform: string;
  memo: string;
}

export interface ServiceRegisterParams {
  serviceType: string;
  description: string;
  priceToken: string; // hex 32-byte
  priceAmount: bigint; // u128
  endpoint: string;
  active: boolean;
}

// --- RPC response types ---

export interface AgentIdentity {
  address: string; // hex
  name: string;
  metadata: Record<string, string>;
  registered_at: number;
}

export interface TokenDef {
  id: string; // hex
  name: string;
  symbol: string;
  decimals: number;
  total_supply: string; // u128 as string
  issuer: string; // hex
}

export interface ReputationAttestation {
  from: string; // hex
  to: string; // hex
  category: string;
  score: number;
  platform: string;
  memo: string;
  block_height: number;
}

export interface ServiceEntry {
  provider: string; // hex
  service_type: string;
  description: string;
  price_token: string; // hex
  price_amount: string; // u128 as string
  endpoint: string;
  active: boolean;
}

export interface BlockInfo {
  height: number;
  prev_hash: string;
  timestamp: number;
  validator: string;
  transactions: unknown[];
  state_root: string;
  hash: string;
}

export interface TransactionReceipt {
  blockHeight: number;
  transactionIndex: number;
}

// --- Client config ---

export interface ClawClientConfig {
  rpcUrl?: string;
  wallet?: WalletLike;
}

export interface WalletLike {
  publicKey: Uint8Array;
  address: string;
  sign(message: Uint8Array): Promise<Uint8Array>;
}
