// ---------------------------------------------------------------------------
// ClawNetwork SDK — Wallet (Ed25519 key management and signing)
// ---------------------------------------------------------------------------

import { ed25519 } from '@noble/curves/ed25519';
import type { WalletLike } from './types.js';

/** Hex-encode a Uint8Array. */
export function toHex(bytes: Uint8Array): string {
  return Array.from(bytes)
    .map((b) => b.toString(16).padStart(2, '0'))
    .join('');
}

/** Decode a hex string to Uint8Array. */
export function fromHex(hex: string): Uint8Array {
  if (hex.length % 2 !== 0) throw new Error('Invalid hex string (odd length)');
  const bytes = new Uint8Array(hex.length / 2);
  for (let i = 0; i < hex.length; i += 2) {
    bytes[i / 2] = parseInt(hex.substring(i, i + 2), 16);
  }
  return bytes;
}

/**
 * Ed25519 wallet for ClawNetwork.
 *
 * Compatible with Rust `ed25519-dalek` crate signatures.
 */
export class Wallet implements WalletLike {
  /** 32-byte Ed25519 private key (seed). */
  readonly privateKey: Uint8Array;
  /** 32-byte Ed25519 public key. */
  readonly publicKey: Uint8Array;
  /** Hex-encoded public key (= on-chain address). */
  readonly address: string;

  private constructor(privateKey: Uint8Array) {
    this.privateKey = privateKey;
    this.publicKey = ed25519.getPublicKey(privateKey);
    this.address = toHex(this.publicKey);
  }

  /** Generate a new random wallet. */
  static generate(): Wallet {
    const privKey = ed25519.utils.randomPrivateKey();
    return new Wallet(privKey);
  }

  /** Restore a wallet from a 32-byte hex private key. */
  static fromPrivateKey(hexOrBytes: string | Uint8Array): Wallet {
    const bytes =
      typeof hexOrBytes === 'string' ? fromHex(hexOrBytes) : hexOrBytes;
    if (bytes.length !== 32) {
      throw new Error(`Private key must be 32 bytes, got ${bytes.length}`);
    }
    return new Wallet(bytes);
  }

  /** Sign a message, returning a 64-byte Ed25519 signature. */
  async sign(message: Uint8Array): Promise<Uint8Array> {
    return ed25519.sign(message, this.privateKey);
  }

  /** Synchronous sign (convenience). */
  signSync(message: Uint8Array): Uint8Array {
    return ed25519.sign(message, this.privateKey);
  }

  /** Verify a signature against a message and public key. */
  static verify(
    signature: Uint8Array,
    message: Uint8Array,
    publicKey: Uint8Array,
  ): boolean {
    return ed25519.verify(signature, message, publicKey);
  }
}
