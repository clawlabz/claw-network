import { describe, it, expect } from 'vitest';
import { Wallet, hexToBytes, bytesToHex } from '../src/core/wallet.js';

describe('hexToBytes / bytesToHex', () => {
  it('should round-trip hex encoding', () => {
    const hex = 'aabbccdd00112233';
    const bytes = hexToBytes(hex);
    expect(bytes.length).toBe(4 + 4);
    expect(bytesToHex(bytes)).toBe(hex);
  });

  it('should handle 0x prefix', () => {
    const bytes = hexToBytes('0xaabb');
    expect(bytesToHex(bytes)).toBe('aabb');
  });

  it('should reject odd-length hex', () => {
    expect(() => hexToBytes('abc')).toThrow('Invalid hex string length');
  });

  it('should reject invalid hex characters', () => {
    expect(() => hexToBytes('zzzz')).toThrow('Invalid hex character');
  });
});

describe('Wallet', () => {
  it('should generate a new wallet with 32-byte keys', async () => {
    const wallet = await Wallet.generate();
    expect(wallet.privateKey.length).toBe(32);
    expect(wallet.publicKey.length).toBe(32);
    expect(wallet.address.length).toBe(64); // hex
  });

  it('should restore from private key', async () => {
    const original = await Wallet.generate();
    const restored = await Wallet.fromPrivateKey(bytesToHex(original.privateKey));
    expect(restored.address).toBe(original.address);
    expect(bytesToHex(restored.publicKey)).toBe(bytesToHex(original.publicKey));
  });

  it('should reject invalid private key length', async () => {
    await expect(Wallet.fromPrivateKey('aabb')).rejects.toThrow('32 bytes');
  });

  it('should sign and verify messages', async () => {
    const wallet = await Wallet.generate();
    const message = new TextEncoder().encode('hello clawnetwork');
    const signature = await wallet.sign(message);
    expect(signature.length).toBe(64);

    const valid = await wallet.verify(message, signature);
    expect(valid).toBe(true);

    // Tamper with message
    const tampered = new TextEncoder().encode('hello tampered');
    const invalid = await wallet.verify(tampered, signature);
    expect(invalid).toBe(false);
  });

  it('should export to JSON', async () => {
    const wallet = await Wallet.generate();
    const json = wallet.toJSON();
    expect(json.privateKey.length).toBe(64);
    expect(json.publicKey.length).toBe(64);
    expect(json.address).toBe(json.publicKey);
  });

  it('should generate unique wallets', async () => {
    const w1 = await Wallet.generate();
    const w2 = await Wallet.generate();
    expect(w1.address).not.toBe(w2.address);
  });
});
