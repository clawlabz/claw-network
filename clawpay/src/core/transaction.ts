/**
 * Transaction building and Borsh serialization.
 *
 * Borsh encoding rules (matching Rust borsh crate):
 * - u8: 1 byte
 * - u64: 8 bytes LE
 * - u128: 16 bytes LE
 * - bool: 1 byte (0 or 1)
 * - Vec<u8>: 4 bytes LE length + data
 * - String: 4 bytes LE length + UTF-8 bytes
 * - [u8; N]: N bytes (fixed array, no length prefix)
 * - Enum: 1 byte discriminant + variant data
 *
 * Transaction struct (Borsh order):
 *   tx_type: u8 (enum discriminant)
 *   from: [u8; 32]
 *   nonce: u64 LE
 *   payload: Vec<u8> (4-byte LE length + data)
 *   signature: [u8; 64]
 *
 * Signable bytes: tx_type(1) || from(32) || nonce(8 LE) || payload(raw, no length prefix)
 */

import { type RawTransaction, type TokenTransferPayload, type ServiceRegisterPayload, TxType, CLAW_DECIMALS, NATIVE_TOKEN_ID } from './types.js';
import { type Wallet, bytesToHex, hexToBytes } from './wallet.js';

// ---------------------------------------------------------------------------
// Borsh serialization helpers
// ---------------------------------------------------------------------------

/** Write a u8. */
function writeU8(buf: number[], value: number): void {
  buf.push(value & 0xff);
}

/** Write a u32 LE. */
function writeU32LE(buf: number[], value: number): void {
  buf.push(value & 0xff);
  buf.push((value >> 8) & 0xff);
  buf.push((value >> 16) & 0xff);
  buf.push((value >> 24) & 0xff);
}

/** Write a u64 LE. */
function writeU64LE(buf: number[], value: bigint): void {
  const lo = Number(value & 0xffffffffn);
  const hi = Number((value >> 32n) & 0xffffffffn);
  writeU32LE(buf, lo);
  writeU32LE(buf, hi);
}

/** Write a u128 LE. */
function writeU128LE(buf: number[], value: bigint): void {
  writeU64LE(buf, value & 0xffffffffffffffffn);
  writeU64LE(buf, (value >> 64n) & 0xffffffffffffffffn);
}

/** Write fixed bytes (no length prefix). */
function writeFixedBytes(buf: number[], data: Uint8Array): void {
  for (const b of data) {
    buf.push(b);
  }
}

/** Write a Vec<u8> (4-byte LE length prefix + data). */
function writeVecU8(buf: number[], data: Uint8Array): void {
  writeU32LE(buf, data.length);
  writeFixedBytes(buf, data);
}

/** Write a Borsh string (4-byte LE length + UTF-8 bytes). */
function writeBorshString(buf: number[], str: string): void {
  const encoded = new TextEncoder().encode(str);
  writeU32LE(buf, encoded.length);
  writeFixedBytes(buf, encoded);
}

/** Write a bool. */
function writeBool(buf: number[], value: boolean): void {
  buf.push(value ? 1 : 0);
}

// ---------------------------------------------------------------------------
// Payload serialization
// ---------------------------------------------------------------------------

/** Serialize TokenTransferPayload to Borsh bytes. */
export function serializeTokenTransferPayload(
  payload: TokenTransferPayload,
): Uint8Array {
  const buf: number[] = [];
  writeFixedBytes(buf, payload.to); // [u8; 32]
  writeU128LE(buf, payload.amount); // u128
  return new Uint8Array(buf);
}

/** Serialize ServiceRegisterPayload to Borsh bytes. */
export function serializeServiceRegisterPayload(
  payload: ServiceRegisterPayload,
): Uint8Array {
  const buf: number[] = [];
  writeBorshString(buf, payload.serviceType);
  writeBorshString(buf, payload.description);
  writeFixedBytes(buf, payload.priceToken); // [u8; 32]
  writeU128LE(buf, payload.priceAmount); // u128
  writeBorshString(buf, payload.endpoint);
  writeBool(buf, payload.active);
  return new Uint8Array(buf);
}

// ---------------------------------------------------------------------------
// Transaction serialization
// ---------------------------------------------------------------------------

/** Build the signable bytes: tx_type(1) || from(32) || nonce(8 LE) || payload(raw). */
export function buildSignableBytes(
  txType: TxType,
  from: Uint8Array,
  nonce: bigint,
  payload: Uint8Array,
): Uint8Array {
  const buf: number[] = [];
  writeU8(buf, txType);
  writeFixedBytes(buf, from);
  writeU64LE(buf, nonce);
  writeFixedBytes(buf, payload); // raw payload, no length prefix
  return new Uint8Array(buf);
}

/** Borsh-serialize a full Transaction struct. */
export function serializeTransaction(tx: RawTransaction): Uint8Array {
  const buf: number[] = [];
  writeU8(buf, tx.txType); // TxType enum discriminant
  writeFixedBytes(buf, tx.from); // [u8; 32]
  writeU64LE(buf, tx.nonce); // u64
  writeVecU8(buf, tx.payload); // Vec<u8>
  writeFixedBytes(buf, tx.signature); // [u8; 64]
  return new Uint8Array(buf);
}

// ---------------------------------------------------------------------------
// Transaction builder
// ---------------------------------------------------------------------------

export interface TransferParams {
  readonly to: string; // hex address
  readonly amount: string; // human-readable (e.g., "10" = 10 CLAW)
  readonly decimals?: number; // defaults to CLAW_DECIMALS (9)
}

/** Parse a human-readable amount string to base units. */
export function parseAmount(amount: string, decimals: number = CLAW_DECIMALS): bigint {
  const parts = amount.split('.');
  const whole = BigInt(parts[0] || '0');
  let fractional = 0n;
  if (parts[1]) {
    const frac = parts[1].padEnd(decimals, '0').slice(0, decimals);
    fractional = BigInt(frac);
  }
  return whole * 10n ** BigInt(decimals) + fractional;
}

/** Format base units to human-readable string. */
export function formatAmount(baseUnits: bigint, decimals: number = CLAW_DECIMALS): string {
  const divisor = 10n ** BigInt(decimals);
  const whole = baseUnits / divisor;
  const frac = baseUnits % divisor;
  if (frac === 0n) return whole.toString();
  const fracStr = frac.toString().padStart(decimals, '0').replace(/0+$/, '');
  return `${whole}.${fracStr}`;
}

/** Build and sign a TokenTransfer transaction. */
export async function buildTransferTx(
  wallet: Wallet,
  nonce: bigint,
  params: TransferParams,
): Promise<{ tx: RawTransaction; hash: string }> {
  const to = hexToBytes(params.to);
  if (to.length !== 32) {
    throw new Error(`Invalid recipient address: expected 32 bytes, got ${to.length}`);
  }

  const decimals = params.decimals ?? CLAW_DECIMALS;
  const amount = parseAmount(params.amount, decimals);
  if (amount <= 0n) {
    throw new Error('Transfer amount must be positive');
  }

  const payload = serializeTokenTransferPayload({ to, amount });
  const signableBytes = buildSignableBytes(TxType.TokenTransfer, wallet.publicKey, nonce, payload);
  const signature = await wallet.sign(signableBytes);

  const tx: RawTransaction = {
    txType: TxType.TokenTransfer,
    from: wallet.publicKey,
    nonce,
    payload,
    signature,
  };

  const serialized = serializeTransaction(tx);
  const txHex = bytesToHex(serialized);

  return { tx, hash: txHex };
}

/** Build and sign a ServiceRegister transaction. */
export async function buildServiceRegisterTx(
  wallet: Wallet,
  nonce: bigint,
  params: ServiceRegisterPayload,
): Promise<{ tx: RawTransaction; hash: string }> {
  const payload = serializeServiceRegisterPayload(params);
  const signableBytes = buildSignableBytes(TxType.ServiceRegister, wallet.publicKey, nonce, payload);
  const signature = await wallet.sign(signableBytes);

  const tx: RawTransaction = {
    txType: TxType.ServiceRegister,
    from: wallet.publicKey,
    nonce,
    payload,
    signature,
  };

  return { tx, hash: bytesToHex(serializeTransaction(tx)) };
}
