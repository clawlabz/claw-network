/**
 * Ed25519 wallet — keypair management and transaction signing.
 * Uses @noble/ed25519 for cryptographic operations.
 */

import * as ed from '@noble/ed25519';

// ---------------------------------------------------------------------------
// Hex utilities (zero-dependency)
// ---------------------------------------------------------------------------

export function hexToBytes(hex: string): Uint8Array {
  const clean = hex.startsWith('0x') ? hex.slice(2) : hex;
  if (clean.length % 2 !== 0) {
    throw new Error(`Invalid hex string length: ${clean.length}`);
  }
  const bytes = new Uint8Array(clean.length / 2);
  for (let i = 0; i < bytes.length; i++) {
    const byte = parseInt(clean.slice(i * 2, i * 2 + 2), 16);
    if (Number.isNaN(byte)) {
      throw new Error(`Invalid hex character at position ${i * 2}`);
    }
    bytes[i] = byte;
  }
  return bytes;
}

export function bytesToHex(bytes: Uint8Array): string {
  return Array.from(bytes)
    .map((b) => b.toString(16).padStart(2, '0'))
    .join('');
}

// ---------------------------------------------------------------------------
// Wallet
// ---------------------------------------------------------------------------

export interface WalletData {
  readonly privateKey: string; // hex, 64 chars
  readonly publicKey: string; // hex, 64 chars
  readonly address: string; // same as publicKey (Ed25519 pubkey IS the address)
}

export class Wallet {
  readonly privateKey: Uint8Array; // 32 bytes
  readonly publicKey: Uint8Array; // 32 bytes

  private constructor(privateKey: Uint8Array, publicKey: Uint8Array) {
    this.privateKey = privateKey;
    this.publicKey = publicKey;
  }

  /** Generate a new random wallet. */
  static async generate(): Promise<Wallet> {
    const privateKey = ed.utils.randomPrivateKey();
    const publicKey = await ed.getPublicKeyAsync(privateKey);
    return new Wallet(privateKey, publicKey);
  }

  /** Restore wallet from a private key hex string. */
  static async fromPrivateKey(hex: string): Promise<Wallet> {
    const privateKey = hexToBytes(hex);
    if (privateKey.length !== 32) {
      throw new Error(`Private key must be 32 bytes, got ${privateKey.length}`);
    }
    const publicKey = await ed.getPublicKeyAsync(privateKey);
    return new Wallet(privateKey, publicKey);
  }

  /** Get the address as hex string (same as public key). */
  get address(): string {
    return bytesToHex(this.publicKey);
  }

  /** Sign arbitrary bytes with the private key. */
  async sign(message: Uint8Array): Promise<Uint8Array> {
    return ed.signAsync(message, this.privateKey);
  }

  /** Verify a signature against a message. */
  async verify(
    message: Uint8Array,
    signature: Uint8Array,
  ): Promise<boolean> {
    return ed.verifyAsync(signature, message, this.publicKey);
  }

  /** Export wallet data as a plain object. */
  toJSON(): WalletData {
    return {
      privateKey: bytesToHex(this.privateKey),
      publicKey: bytesToHex(this.publicKey),
      address: this.address,
    };
  }
}
