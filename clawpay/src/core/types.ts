/**
 * Core type definitions for ClawPay SDK.
 * Mirrors ClawNetwork chain types for TypeScript.
 */

// ---------------------------------------------------------------------------
// Transaction types (matches Rust TxType enum discriminant)
// ---------------------------------------------------------------------------

export const TxType = {
  AgentRegister: 0,
  TokenTransfer: 1,
  TokenCreate: 2,
  TokenMintTransfer: 3,
  ReputationAttest: 4,
  ServiceRegister: 5,
  ContractDeploy: 6,
  ContractCall: 7,
  StakeDeposit: 8,
  StakeWithdraw: 9,
  StakeClaim: 10,
  PlatformActivityReport: 11,
  TokenApprove: 12,
  TokenBurn: 13,
  ChangeDelegation: 14,
  MinerRegister: 15,
  MinerHeartbeat: 16,
  ContractUpgradeAnnounce: 17,
  ContractUpgradeExecute: 18,
} as const;

export type TxType = (typeof TxType)[keyof typeof TxType];

// ---------------------------------------------------------------------------
// Chain constants
// ---------------------------------------------------------------------------

/** Native CLAW token ID (all zeros, represents the native token). */
export const NATIVE_TOKEN_ID = new Uint8Array(32);

/** CLAW token decimals. */
export const CLAW_DECIMALS = 9;

/** @deprecated Use CLAW_DECIMALS */
export const CLW_DECIMALS = CLAW_DECIMALS;

/** Gas fee per transaction in base units (0.001 CLAW). */
export const GAS_FEE = 1_000_000n;

/** Default RPC endpoints. */
export const RPC_MAINNET = 'https://rpc.clawlabz.xyz';
export const RPC_TESTNET = 'https://testnet-rpc.clawlabz.xyz';

// ---------------------------------------------------------------------------
// Transaction structure
// ---------------------------------------------------------------------------

export interface RawTransaction {
  readonly txType: TxType;
  readonly from: Uint8Array; // 32 bytes
  readonly nonce: bigint;
  readonly payload: Uint8Array;
  readonly signature: Uint8Array; // 64 bytes
}

// ---------------------------------------------------------------------------
// Payload types
// ---------------------------------------------------------------------------

export interface TokenTransferPayload {
  readonly to: Uint8Array; // 32 bytes
  readonly amount: bigint;
}

export interface AgentRegisterPayload {
  readonly name: string;
  readonly metadata: ReadonlyMap<string, string>;
}

export interface ServiceRegisterPayload {
  readonly serviceType: string;
  readonly description: string;
  readonly priceToken: Uint8Array; // 32 bytes
  readonly priceAmount: bigint;
  readonly endpoint: string;
  readonly active: boolean;
}

// ---------------------------------------------------------------------------
// RPC response types
// ---------------------------------------------------------------------------

export interface TransactionReceipt {
  readonly blockHeight: number;
  readonly transactionIndex: number;
}

export interface TransactionInfo {
  readonly hash: string;
  readonly txType: number;
  readonly typeName: string;
  readonly from: string;
  readonly to: string | null;
  readonly amount: string | null;
  readonly nonce: number;
  readonly blockHeight: number;
  readonly timestamp: number;
  readonly fee: string;
}

export interface AgentIdentity {
  readonly address: string;
  readonly name: string;
  readonly metadata: Record<string, string>;
  readonly registered_at: number;
}

export interface ServiceEntry {
  readonly provider: string;
  readonly service_type: string;
  readonly description: string;
  readonly price_token: string;
  readonly price_amount: string;
  readonly endpoint: string;
  readonly active: boolean;
}

// ---------------------------------------------------------------------------
// SDK configuration
// ---------------------------------------------------------------------------

export interface ClawPayConfig {
  /** Ed25519 private key as hex string (64 chars = 32 bytes). */
  readonly privateKey: string;
  /** RPC endpoint URL. Defaults to mainnet. */
  readonly rpc?: string;
  /** Maximum number of retries for RPC calls. Defaults to 3. */
  readonly maxRetries?: number;
  /** Timeout for RPC calls in milliseconds. Defaults to 10000. */
  readonly timeout?: number;
}

// ---------------------------------------------------------------------------
// HTTP 402 protocol types
// ---------------------------------------------------------------------------

export interface PayChallenge {
  readonly challenge_id: string;
  readonly recipient: string;
  readonly amount: string;
  readonly token: string;
  readonly chain: string;
  readonly expires_at: number;
}

export interface PayCredential {
  readonly challenge_id: string;
  readonly tx_hash: string;
}

export interface PayReceipt {
  readonly tx_hash: string;
  readonly block_height: number;
  readonly settled: boolean;
}

// ---------------------------------------------------------------------------
// Middleware types
// ---------------------------------------------------------------------------

export interface ChargeOptions {
  /** Amount in human-readable units (e.g., "10" = 10 CLAW). */
  readonly amount: string;
  /** Token symbol. Defaults to "CLAW" (native). */
  readonly token?: string;
  /** Challenge expiry in seconds. Defaults to 300 (5 minutes). */
  readonly expiresIn?: number;
}
