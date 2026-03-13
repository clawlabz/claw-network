// ---------------------------------------------------------------------------
// @clawlabz/clawnetwork-sdk — Unit tests
// ---------------------------------------------------------------------------

import { describe, it, expect } from 'vitest';
import {
  Wallet,
  toHex,
  fromHex,
  TxType,
  signableBytes,
  serializeTransaction,
  encodeAgentRegisterPayload,
  encodeTokenTransferPayload,
  encodeTokenCreatePayload,
  encodeTokenMintTransferPayload,
  encodeReputationAttestPayload,
  encodeServiceRegisterPayload,
  transactionHash,
  transactionHashHex,
} from '../src/index.js';

// ---------------------------------------------------------------------------
// Hex utils
// ---------------------------------------------------------------------------

describe('hex utils', () => {
  it('toHex / fromHex roundtrip', () => {
    const bytes = new Uint8Array([0, 1, 127, 128, 255]);
    expect(fromHex(toHex(bytes))).toEqual(bytes);
  });

  it('fromHex rejects odd-length', () => {
    expect(() => fromHex('abc')).toThrow('odd length');
  });

  it('toHex produces lowercase', () => {
    expect(toHex(new Uint8Array([0xab, 0xcd]))).toBe('abcd');
  });
});

// ---------------------------------------------------------------------------
// Wallet
// ---------------------------------------------------------------------------

describe('Wallet', () => {
  it('generate produces a valid wallet', () => {
    const w = Wallet.generate();
    expect(w.privateKey).toHaveLength(32);
    expect(w.publicKey).toHaveLength(32);
    expect(w.address).toHaveLength(64); // 32 bytes hex
  });

  it('fromPrivateKey roundtrip', () => {
    const w1 = Wallet.generate();
    const w2 = Wallet.fromPrivateKey(toHex(w1.privateKey));
    expect(w2.address).toBe(w1.address);
    expect(toHex(w2.publicKey)).toBe(toHex(w1.publicKey));
  });

  it('fromPrivateKey accepts Uint8Array', () => {
    const w1 = Wallet.generate();
    const w2 = Wallet.fromPrivateKey(w1.privateKey);
    expect(w2.address).toBe(w1.address);
  });

  it('fromPrivateKey rejects wrong size', () => {
    expect(() => Wallet.fromPrivateKey('aabb')).toThrow('32 bytes');
  });

  it('sign produces a 64-byte signature', async () => {
    const w = Wallet.generate();
    const msg = new Uint8Array([1, 2, 3]);
    const sig = await w.sign(msg);
    expect(sig).toHaveLength(64);
  });

  it('signSync produces a 64-byte signature', () => {
    const w = Wallet.generate();
    const msg = new Uint8Array([1, 2, 3]);
    const sig = w.signSync(msg);
    expect(sig).toHaveLength(64);
  });

  it('sign and verify roundtrip', async () => {
    const w = Wallet.generate();
    const msg = new TextEncoder().encode('hello claw');
    const sig = await w.sign(msg);
    expect(Wallet.verify(sig, msg, w.publicKey)).toBe(true);
  });

  it('verify rejects tampered message', async () => {
    const w = Wallet.generate();
    const msg = new TextEncoder().encode('original');
    const sig = await w.sign(msg);
    const tampered = new TextEncoder().encode('tampered');
    expect(Wallet.verify(sig, tampered, w.publicKey)).toBe(false);
  });

  it('two wallets produce different addresses', () => {
    const w1 = Wallet.generate();
    const w2 = Wallet.generate();
    expect(w1.address).not.toBe(w2.address);
  });
});

// ---------------------------------------------------------------------------
// Serialization — payload encoders
// ---------------------------------------------------------------------------

describe('payload serialization', () => {
  it('AgentRegister payload has correct structure', () => {
    const payload = encodeAgentRegisterPayload('test', { key: 'val' });
    // string "test": 4 bytes len (4) + 4 bytes = 8
    // map count: 4 bytes (1 entry)
    // key "key": 4 + 3 = 7
    // val "val": 4 + 3 = 7
    // total = 8 + 4 + 7 + 7 = 26
    expect(payload).toHaveLength(26);
    // First 4 bytes: string length 4 (LE)
    expect(payload[0]).toBe(4);
    expect(payload[1]).toBe(0);
    expect(payload[2]).toBe(0);
    expect(payload[3]).toBe(0);
  });

  it('AgentRegister empty metadata', () => {
    const payload = encodeAgentRegisterPayload('a', {});
    // string "a": 4 + 1 = 5, map: 4 (count 0) = 4. Total = 9
    expect(payload).toHaveLength(9);
  });

  it('AgentRegister sorts metadata keys (BTreeMap compat)', () => {
    const payload = encodeAgentRegisterPayload('x', { z: '1', a: '2' });
    // Decode: skip name (4+1=5), map count at offset 5 = 2
    expect(payload[5]).toBe(2);
    // First key should be "a" (sorted)
    // key length at offset 9
    expect(payload[9]).toBe(1); // "a" length
    expect(payload[13]).toBe(0x61); // 'a'
  });

  it('TokenTransfer payload is 48 bytes (32 + 16)', () => {
    const to = new Uint8Array(32).fill(0xab);
    const payload = encodeTokenTransferPayload(to, 1000n);
    expect(payload).toHaveLength(48);
    // First 32 bytes are the address
    expect(payload[0]).toBe(0xab);
    expect(payload[31]).toBe(0xab);
    // Amount: 1000 = 0x03E8, LE
    expect(payload[32]).toBe(0xe8);
    expect(payload[33]).toBe(0x03);
  });

  it('TokenCreate payload encodes correctly', () => {
    const payload = encodeTokenCreatePayload('Tok', 'TK', 9, 1000000n);
    // "Tok": 4+3=7, "TK": 4+2=6, decimals: 1, total_supply: 16. Total = 30
    expect(payload).toHaveLength(30);
  });

  it('TokenMintTransfer payload is 80 bytes (32+32+16)', () => {
    const tokenId = new Uint8Array(32).fill(1);
    const to = new Uint8Array(32).fill(2);
    const payload = encodeTokenMintTransferPayload(tokenId, to, 500n);
    expect(payload).toHaveLength(80);
  });

  it('ReputationAttest payload encodes correctly', () => {
    const to = new Uint8Array(32).fill(0);
    const payload = encodeReputationAttestPayload(to, 'game', 50, 'arena', '');
    // to: 32, "game": 4+4=8, score i16: 2, "arena": 4+5=9, "": 4+0=4
    // Total = 32 + 8 + 2 + 9 + 4 = 55
    expect(payload).toHaveLength(55);
  });

  it('ReputationAttest negative score encodes correctly', () => {
    const to = new Uint8Array(32).fill(0);
    const payload = encodeReputationAttestPayload(
      to,
      'x',
      -100,
      'p',
      '',
    );
    // i16(-100) = 0xFF9C in unsigned LE = [0x9C, 0xFF]
    // Offset: 32 (to) + 5 ("x" = 4+1) = 37
    expect(payload[37]).toBe(0x9c);
    expect(payload[38]).toBe(0xff);
  });

  it('ServiceRegister payload encodes correctly', () => {
    const priceToken = new Uint8Array(32).fill(0);
    const payload = encodeServiceRegisterPayload(
      'svc',
      'desc',
      priceToken,
      100n,
      'http://x',
      true,
    );
    // "svc":4+3=7, "desc":4+4=8, token:32, amount:16, "http://x":4+8=12, bool:1
    // Total = 7 + 8 + 32 + 16 + 12 + 1 = 76
    expect(payload).toHaveLength(76);
    // Last byte = bool true = 1
    expect(payload[payload.length - 1]).toBe(1);
  });
});

// ---------------------------------------------------------------------------
// Serialization — Transaction
// ---------------------------------------------------------------------------

describe('transaction serialization', () => {
  const wallet = Wallet.generate();

  function makeTx(payload: Uint8Array = new Uint8Array([1, 2, 3])): {
    txType: number;
    from: Uint8Array;
    nonce: bigint;
    payload: Uint8Array;
    signature: Uint8Array;
  } {
    return {
      txType: TxType.TokenTransfer,
      from: wallet.publicKey,
      nonce: 1n,
      payload,
      signature: new Uint8Array(64),
    };
  }

  it('signableBytes has correct length', () => {
    const tx = makeTx();
    const sb = signableBytes(tx);
    // 1 (txType) + 32 (from) + 8 (nonce) + 3 (payload) = 44
    expect(sb).toHaveLength(44);
  });

  it('signableBytes starts with tx_type byte', () => {
    const tx = makeTx();
    const sb = signableBytes(tx);
    expect(sb[0]).toBe(TxType.TokenTransfer); // 1
  });

  it('serializeTransaction has correct length', () => {
    const tx = makeTx();
    const serialized = serializeTransaction(tx);
    // 1 (txType) + 32 (from) + 8 (nonce) + 4 (vec len) + 3 (payload) + 64 (sig) = 112
    expect(serialized).toHaveLength(112);
  });

  it('serializeTransaction Vec<u8> has length prefix', () => {
    const tx = makeTx(new Uint8Array([0xaa, 0xbb]));
    const serialized = serializeTransaction(tx);
    // After txType(1) + from(32) + nonce(8) = offset 41
    // Vec length = 2 (LE u32)
    expect(serialized[41]).toBe(2);
    expect(serialized[42]).toBe(0);
    expect(serialized[43]).toBe(0);
    expect(serialized[44]).toBe(0);
    // Data
    expect(serialized[45]).toBe(0xaa);
    expect(serialized[46]).toBe(0xbb);
  });
});

// ---------------------------------------------------------------------------
// Signing + Transaction flow
// ---------------------------------------------------------------------------

describe('sign transaction flow', () => {
  it('signed transaction verifies', async () => {
    const wallet = Wallet.generate();
    const payload = encodeTokenTransferPayload(
      new Uint8Array(32).fill(1),
      500n,
    );

    const tx = {
      txType: TxType.TokenTransfer as TxType,
      from: wallet.publicKey,
      nonce: 1n,
      payload,
      signature: new Uint8Array(64),
    };

    const msg = signableBytes(tx);
    tx.signature = await wallet.sign(msg);

    // Verify
    expect(Wallet.verify(tx.signature, msg, tx.from)).toBe(true);
  });

  it('tampered payload fails verification', async () => {
    const wallet = Wallet.generate();
    const payload = encodeTokenTransferPayload(
      new Uint8Array(32).fill(1),
      500n,
    );

    const tx = {
      txType: TxType.TokenTransfer as TxType,
      from: wallet.publicKey,
      nonce: 1n,
      payload,
      signature: new Uint8Array(64),
    };

    const msg = signableBytes(tx);
    tx.signature = await wallet.sign(msg);

    // Tamper
    tx.payload = new Uint8Array([99]);
    const tamperedMsg = signableBytes(tx);
    expect(Wallet.verify(tx.signature, tamperedMsg, tx.from)).toBe(false);
  });
});

// ---------------------------------------------------------------------------
// Hash (blake3)
// ---------------------------------------------------------------------------

describe('transaction hash', () => {
  it('produces a 32-byte hash', () => {
    const wallet = Wallet.generate();
    const tx = {
      txType: TxType.AgentRegister as TxType,
      from: wallet.publicKey,
      nonce: 0n,
      payload: encodeAgentRegisterPayload('test', {}),
      signature: new Uint8Array(64),
    };

    const hash = transactionHash(tx);
    expect(hash).toHaveLength(32);
  });

  it('transactionHashHex returns 64-char hex string', () => {
    const wallet = Wallet.generate();
    const tx = {
      txType: TxType.AgentRegister as TxType,
      from: wallet.publicKey,
      nonce: 0n,
      payload: encodeAgentRegisterPayload('test', {}),
      signature: new Uint8Array(64),
    };

    const hex = transactionHashHex(tx);
    expect(hex).toHaveLength(64);
    expect(/^[0-9a-f]{64}$/.test(hex)).toBe(true);
  });

  it('different transactions produce different hashes', () => {
    const wallet = Wallet.generate();
    const tx1 = {
      txType: TxType.AgentRegister as TxType,
      from: wallet.publicKey,
      nonce: 0n,
      payload: encodeAgentRegisterPayload('agent1', {}),
      signature: new Uint8Array(64),
    };
    const tx2 = {
      txType: TxType.AgentRegister as TxType,
      from: wallet.publicKey,
      nonce: 1n,
      payload: encodeAgentRegisterPayload('agent2', {}),
      signature: new Uint8Array(64),
    };

    expect(transactionHashHex(tx1)).not.toBe(transactionHashHex(tx2));
  });
});

// ---------------------------------------------------------------------------
// u128 encoding
// ---------------------------------------------------------------------------

describe('u128 encoding', () => {
  it('encodes large amounts correctly', () => {
    const amount = 1000000000000000000n; // 10^18
    const to = new Uint8Array(32).fill(0);
    const payload = encodeTokenTransferPayload(to, amount);
    // Extract the 16-byte u128 at offset 32
    const u128Bytes = payload.slice(32, 48);
    // Reconstruct
    let val = 0n;
    for (let i = 0; i < 16; i++) {
      val |= BigInt(u128Bytes[i]!) << BigInt(i * 8);
    }
    expect(val).toBe(amount);
  });

  it('encodes zero correctly', () => {
    const to = new Uint8Array(32).fill(0);
    const payload = encodeTokenTransferPayload(to, 0n);
    const u128Bytes = payload.slice(32, 48);
    expect(u128Bytes.every((b) => b === 0)).toBe(true);
  });

  it('encodes max u128 correctly', () => {
    const maxU128 = (1n << 128n) - 1n;
    const to = new Uint8Array(32).fill(0);
    const payload = encodeTokenTransferPayload(to, maxU128);
    const u128Bytes = payload.slice(32, 48);
    expect(u128Bytes.every((b) => b === 0xff)).toBe(true);
  });
});
