// ---------------------------------------------------------------------------
// ClawNetwork SDK — Borsh serialization (byte-compatible with Rust `borsh`)
// ---------------------------------------------------------------------------
//
// We use manual borsh encoding to guarantee exact byte-level compatibility
// with the Rust crate. The `borsh` npm package's schema API can be fragile
// across versions, so we encode directly.
//
// Borsh spec reference:
//   string  = 4-byte LE length + UTF-8 bytes
//   u8      = 1 byte
//   i16     = 2 bytes LE (signed)
//   u64     = 8 bytes LE
//   u128    = 16 bytes LE
//   [u8; N] = N raw bytes (no prefix)
//   Vec<u8> = 4-byte LE length + raw bytes
//   bool    = 1 byte (0 or 1)
//   BTreeMap<String,String> = 4-byte LE count + repeated (key, value) pairs
//     (Rust BTreeMap serializes in sorted-key order)
// ---------------------------------------------------------------------------

import { TxType } from './types.js';
import type { Transaction } from './types.js';

// ---- Primitive writers ----

function writeU8(buf: number[], v: number): void {
  buf.push(v & 0xff);
}

function writeU16LE(buf: number[], v: number): void {
  buf.push(v & 0xff, (v >> 8) & 0xff);
}

function writeI16LE(buf: number[], v: number): void {
  // Reinterpret as unsigned 16-bit for LE encoding
  const u = v < 0 ? v + 0x10000 : v;
  writeU16LE(buf, u);
}

function writeU32LE(buf: number[], v: number): void {
  buf.push(v & 0xff, (v >> 8) & 0xff, (v >> 16) & 0xff, (v >> 24) & 0xff);
}

function writeU64LE(buf: number[], v: bigint): void {
  const lo = Number(v & 0xffffffffn);
  const hi = Number((v >> 32n) & 0xffffffffn);
  writeU32LE(buf, lo);
  writeU32LE(buf, hi);
}

function writeU128LE(buf: number[], v: bigint): void {
  writeU64LE(buf, v & 0xffffffffffffffffn);
  writeU64LE(buf, (v >> 64n) & 0xffffffffffffffffn);
}

function writeFixedBytes(buf: number[], bytes: Uint8Array): void {
  for (let i = 0; i < bytes.length; i++) {
    buf.push(bytes[i]!);
  }
}

function writeString(buf: number[], s: string): void {
  const encoded = new TextEncoder().encode(s);
  writeU32LE(buf, encoded.length);
  writeFixedBytes(buf, encoded);
}

function writeVecU8(buf: number[], data: Uint8Array): void {
  writeU32LE(buf, data.length);
  writeFixedBytes(buf, data);
}

function writeBool(buf: number[], v: boolean): void {
  buf.push(v ? 1 : 0);
}

/**
 * Write a BTreeMap<String, String> in Rust BTreeMap borsh order (sorted keys).
 */
function writeStringMap(buf: number[], map: Record<string, string> | undefined): void {
  const m = map ?? {};
  const keys = Object.keys(m).sort(); // BTreeMap = sorted
  writeU32LE(buf, keys.length);
  for (const key of keys) {
    writeString(buf, key);
    writeString(buf, m[key]!);
  }
}

// ---- Payload encoders ----

export function encodeAgentRegisterPayload(
  name: string,
  metadata: Record<string, string>,
): Uint8Array {
  const buf: number[] = [];
  writeString(buf, name);
  writeStringMap(buf, metadata);
  return new Uint8Array(buf);
}

export function encodeTokenTransferPayload(
  to: Uint8Array,
  amount: bigint,
): Uint8Array {
  const buf: number[] = [];
  writeFixedBytes(buf, to); // [u8; 32]
  writeU128LE(buf, amount);
  return new Uint8Array(buf);
}

export function encodeTokenCreatePayload(
  name: string,
  symbol: string,
  decimals: number,
  totalSupply: bigint,
): Uint8Array {
  const buf: number[] = [];
  writeString(buf, name);
  writeString(buf, symbol);
  writeU8(buf, decimals);
  writeU128LE(buf, totalSupply);
  return new Uint8Array(buf);
}

export function encodeTokenMintTransferPayload(
  tokenId: Uint8Array,
  to: Uint8Array,
  amount: bigint,
): Uint8Array {
  const buf: number[] = [];
  writeFixedBytes(buf, tokenId); // [u8; 32]
  writeFixedBytes(buf, to); // [u8; 32]
  writeU128LE(buf, amount);
  return new Uint8Array(buf);
}

export function encodeReputationAttestPayload(
  to: Uint8Array,
  category: string,
  score: number,
  platform: string,
  memo: string,
): Uint8Array {
  const buf: number[] = [];
  writeFixedBytes(buf, to); // [u8; 32]
  writeString(buf, category);
  writeI16LE(buf, score);
  writeString(buf, platform);
  writeString(buf, memo);
  return new Uint8Array(buf);
}

export function encodeServiceRegisterPayload(
  serviceType: string,
  description: string,
  priceToken: Uint8Array,
  priceAmount: bigint,
  endpoint: string,
  active: boolean,
): Uint8Array {
  const buf: number[] = [];
  writeString(buf, serviceType);
  writeString(buf, description);
  writeFixedBytes(buf, priceToken); // [u8; 32]
  writeU128LE(buf, priceAmount);
  writeString(buf, endpoint);
  writeBool(buf, active);
  return new Uint8Array(buf);
}

// ---- Transaction serialization ----

/**
 * Compute the signable bytes for a transaction:
 *   tx_type(1) || from(32) || nonce(8 LE) || payload(raw, no length prefix)
 *
 * This matches `Transaction::signable_bytes()` in Rust.
 */
export function signableBytes(tx: Transaction): Uint8Array {
  const buf: number[] = [];
  writeU8(buf, tx.txType);
  writeFixedBytes(buf, tx.from);
  writeU64LE(buf, tx.nonce);
  writeFixedBytes(buf, tx.payload); // raw bytes, no Vec prefix
  return new Uint8Array(buf);
}

/**
 * Borsh-serialize a full Transaction struct.
 *
 * Layout (matches Rust `BorshSerialize` derive on Transaction):
 *   tx_type: u8 (enum discriminant)
 *   from:    [u8; 32]
 *   nonce:   u64 LE
 *   payload: Vec<u8> (4-byte LE length + data)
 *   signature: [u8; 64]
 */
export function serializeTransaction(tx: Transaction): Uint8Array {
  const buf: number[] = [];
  writeU8(buf, tx.txType);
  writeFixedBytes(buf, tx.from);
  writeU64LE(buf, tx.nonce);
  writeVecU8(buf, tx.payload); // Vec<u8> with length prefix
  writeFixedBytes(buf, tx.signature); // [u8; 64] fixed
  return new Uint8Array(buf);
}
