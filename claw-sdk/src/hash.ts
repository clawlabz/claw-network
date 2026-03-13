// ---------------------------------------------------------------------------
// ClawNetwork SDK — blake3 transaction hashing
// ---------------------------------------------------------------------------

import { blake3 } from '@noble/hashes/blake3.js';
import { serializeTransaction } from './serialization.js';
import type { Transaction } from './types.js';
import { toHex } from './wallet.js';

/**
 * Compute the transaction hash: blake3(borsh_serialize(tx)).
 *
 * Returns the 32-byte hash as a Uint8Array.
 * Matches `Transaction::hash()` in Rust.
 */
export function transactionHash(tx: Transaction): Uint8Array {
  const serialized = serializeTransaction(tx);
  return blake3(serialized);
}

/**
 * Compute the transaction hash and return it as a hex string.
 */
export function transactionHashHex(tx: Transaction): string {
  return toHex(transactionHash(tx));
}
