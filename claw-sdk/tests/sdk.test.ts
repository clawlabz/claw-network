// ---------------------------------------------------------------------------
// @clawlabz/clawnetwork-sdk — Unit tests
// ---------------------------------------------------------------------------

import { describe, it, expect, vi } from 'vitest';
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
  encodeStakeDepositPayload,
  encodeStakeWithdrawPayload,
  encodeStakeClaimPayload,
  encodeChangeDelegationPayload,
  encodeContractDeployPayload,
  encodeContractCallPayload,
  encodeMinerRegisterPayload,
  encodeMinerHeartbeatPayload,
  transactionHash,
  transactionHashHex,
  ClawClient,
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

// ---------------------------------------------------------------------------
// Phase 2 Serializers — Staking
// ---------------------------------------------------------------------------

describe('encodeStakeDepositPayload', () => {
  it('produces 50 bytes (16 + 32 + 2)', () => {
    const validator = new Uint8Array(32).fill(0xaa);
    const payload = encodeStakeDepositPayload(1000n, validator, 100);
    expect(payload).toHaveLength(50);
  });

  it('encodes amount (u128) in first 16 bytes as LE', () => {
    const validator = new Uint8Array(32).fill(0);
    const amount = 0x0102030405060708090a0b0c0d0e0f10n;
    const payload = encodeStakeDepositPayload(amount, validator, 0);
    // First 16 bytes = amount in LE
    const amountBytes = payload.slice(0, 16);
    let decoded = 0n;
    for (let i = 0; i < 16; i++) {
      decoded |= BigInt(amountBytes[i]!) << BigInt(i * 8);
    }
    expect(decoded).toBe(amount);
  });

  it('encodes validator in bytes 16-48', () => {
    const validator = new Uint8Array(32).fill(0xbb);
    const payload = encodeStakeDepositPayload(500n, validator, 50);
    expect(payload.slice(16, 48)).toEqual(validator);
  });

  it('encodes commissionBps (u16) in last 2 bytes as LE', () => {
    const validator = new Uint8Array(32).fill(0);
    const payload = encodeStakeDepositPayload(100n, validator, 0x1234);
    expect(payload[48]).toBe(0x34);
    expect(payload[49]).toBe(0x12);
  });

  it('handles commission 0', () => {
    const validator = new Uint8Array(32).fill(1);
    const payload = encodeStakeDepositPayload(1000n, validator, 0);
    expect(payload[48]).toBe(0);
    expect(payload[49]).toBe(0);
  });

  it('handles commission max u16', () => {
    const validator = new Uint8Array(32).fill(2);
    const payload = encodeStakeDepositPayload(5000n, validator, 0xffff);
    expect(payload[48]).toBe(0xff);
    expect(payload[49]).toBe(0xff);
  });
});

describe('encodeStakeWithdrawPayload', () => {
  it('produces 48 bytes (16 + 32)', () => {
    const validator = new Uint8Array(32).fill(0xcc);
    const payload = encodeStakeWithdrawPayload(2000n, validator);
    expect(payload).toHaveLength(48);
  });

  it('encodes amount in first 16 bytes as LE u128', () => {
    const validator = new Uint8Array(32).fill(0);
    const amount = 1000000000000000000n; // 10^18
    const payload = encodeStakeWithdrawPayload(amount, validator);
    const amountBytes = payload.slice(0, 16);
    let decoded = 0n;
    for (let i = 0; i < 16; i++) {
      decoded |= BigInt(amountBytes[i]!) << BigInt(i * 8);
    }
    expect(decoded).toBe(amount);
  });

  it('encodes validator in bytes 16-48', () => {
    const validator = new Uint8Array(32).fill(0xdd);
    const payload = encodeStakeWithdrawPayload(500n, validator);
    expect(payload.slice(16, 48)).toEqual(validator);
  });

  it('handles zero amount', () => {
    const validator = new Uint8Array(32).fill(0xff);
    const payload = encodeStakeWithdrawPayload(0n, validator);
    expect(payload.slice(0, 16).every((b) => b === 0)).toBe(true);
    expect(payload.slice(16, 48)).toEqual(validator);
  });
});

describe('encodeStakeClaimPayload', () => {
  it('produces empty payload (0 bytes)', () => {
    const payload = encodeStakeClaimPayload();
    expect(payload).toHaveLength(0);
  });

  it('returns Uint8Array instance', () => {
    const payload = encodeStakeClaimPayload();
    expect(payload).toBeInstanceOf(Uint8Array);
  });
});

describe('encodeChangeDelegationPayload', () => {
  it('produces 66 bytes (32 + 32 + 2)', () => {
    const validator = new Uint8Array(32).fill(1);
    const newOwner = new Uint8Array(32).fill(2);
    const payload = encodeChangeDelegationPayload(validator, newOwner, 200);
    expect(payload).toHaveLength(66);
  });

  it('encodes validator in first 32 bytes', () => {
    const validator = new Uint8Array(32).fill(0xaa);
    const newOwner = new Uint8Array(32).fill(0xbb);
    const payload = encodeChangeDelegationPayload(validator, newOwner, 100);
    expect(payload.slice(0, 32)).toEqual(validator);
  });

  it('encodes newOwner in bytes 32-64', () => {
    const validator = new Uint8Array(32).fill(0xaa);
    const newOwner = new Uint8Array(32).fill(0xbb);
    const payload = encodeChangeDelegationPayload(validator, newOwner, 100);
    expect(payload.slice(32, 64)).toEqual(newOwner);
  });

  it('encodes commissionBps (u16) in last 2 bytes as LE', () => {
    const validator = new Uint8Array(32).fill(0);
    const newOwner = new Uint8Array(32).fill(0);
    const payload = encodeChangeDelegationPayload(validator, newOwner, 0x5678);
    expect(payload[64]).toBe(0x78);
    expect(payload[65]).toBe(0x56);
  });
});

// ---------------------------------------------------------------------------
// Phase 2 Serializers — Contracts
// ---------------------------------------------------------------------------

describe('encodeContractDeployPayload', () => {
  it('produces correct length with known inputs', () => {
    const code = new Uint8Array([0x01, 0x02]);
    const initMethod = 'init';
    const initArgs = new Uint8Array([0x03, 0x04]);
    const payload = encodeContractDeployPayload(code, initMethod, initArgs);
    // code: 4 (len) + 2 (data) = 6
    // method: 4 (len) + 4 (chars) = 8
    // args: 4 (len) + 2 (data) = 6
    // Total = 20
    expect(payload).toHaveLength(20);
  });

  it('encodes code with Vec<u8> length prefix', () => {
    const code = new Uint8Array([0xaa, 0xbb, 0xcc]);
    const initMethod = 'x';
    const initArgs = new Uint8Array([]);
    const payload = encodeContractDeployPayload(code, initMethod, initArgs);
    // First 4 bytes: length 3 (LE)
    expect(payload[0]).toBe(3);
    expect(payload[1]).toBe(0);
    expect(payload[2]).toBe(0);
    expect(payload[3]).toBe(0);
    // Next 3 bytes: code
    expect(payload[4]).toBe(0xaa);
    expect(payload[5]).toBe(0xbb);
    expect(payload[6]).toBe(0xcc);
  });

  it('encodes method as string (length-prefixed)', () => {
    const code = new Uint8Array([]);
    const initMethod = 'setup';
    const initArgs = new Uint8Array([]);
    const payload = encodeContractDeployPayload(code, initMethod, initArgs);
    // Offset 4: code length prefix (0) + code (0)
    // Offset 4: method length prefix
    expect(payload[4]).toBe(5); // "setup" length
    expect(payload[5]).toBe(0);
    expect(payload[6]).toBe(0);
    expect(payload[7]).toBe(0);
    // Method bytes
    expect(payload[8]).toBe(0x73); // 's'
    expect(payload[9]).toBe(0x65); // 'e'
  });

  it('encodes empty code and args', () => {
    const payload = encodeContractDeployPayload(
      new Uint8Array([]),
      'x',
      new Uint8Array([]),
    );
    // 4 (code len) + 4 (method len) + 1 (method) + 4 (args len) = 13
    expect(payload).toHaveLength(13);
  });
});

describe('encodeContractCallPayload', () => {
  it('produces correct length with known inputs', () => {
    const contract = new Uint8Array(32).fill(1);
    const method = 'call';
    const args = new Uint8Array([0x05]);
    const payload = encodeContractCallPayload(contract, method, args, 100n);
    // contract: 32, method: 4+4=8, args: 4+1=5, value: 16
    // Total = 32 + 8 + 5 + 16 = 61
    expect(payload).toHaveLength(61);
  });

  it('encodes contract address in first 32 bytes', () => {
    const contract = new Uint8Array(32).fill(0xee);
    const payload = encodeContractCallPayload(
      contract,
      'x',
      new Uint8Array([]),
      0n,
    );
    expect(payload.slice(0, 32)).toEqual(contract);
  });

  it('encodes method as string after contract', () => {
    const contract = new Uint8Array(32).fill(0);
    const method = 'foo';
    const payload = encodeContractCallPayload(
      contract,
      method,
      new Uint8Array([]),
      0n,
    );
    // Offset 32: method length (3) as u32 LE
    expect(payload[32]).toBe(3);
    expect(payload[33]).toBe(0);
    expect(payload[34]).toBe(0);
    expect(payload[35]).toBe(0);
    // Method bytes
    expect(payload[36]).toBe(0x66); // 'f'
    expect(payload[37]).toBe(0x6f); // 'o'
    expect(payload[38]).toBe(0x6f); // 'o'
  });

  it('encodes args as Vec<u8>', () => {
    const contract = new Uint8Array(32).fill(0);
    const args = new Uint8Array([0x11, 0x22]);
    const payload = encodeContractCallPayload(
      contract,
      'f',
      args,
      0n,
    );
    // Offset 32: method length 1, method "f" (1 byte)
    // Offset 37: args length (2) as u32 LE
    expect(payload[37]).toBe(2);
    expect(payload[38]).toBe(0);
    expect(payload[39]).toBe(0);
    expect(payload[40]).toBe(0);
    expect(payload[41]).toBe(0x11);
    expect(payload[42]).toBe(0x22);
  });

  it('encodes value (u128) at end', () => {
    const contract = new Uint8Array(32).fill(0);
    const payload = encodeContractCallPayload(
      contract,
      'x',
      new Uint8Array([]),
      0x0102030405060708090a0b0c0d0e0f10n,
    );
    // Payload: contract(32) + method_len(4) + method(1) + args_len(4) + args(0) + value(16)
    // Value starts at offset 32+4+1+4 = 41, ends at 41+16 = 57
    const valueBytes = payload.slice(41, 57);
    let decoded = 0n;
    for (let i = 0; i < 16; i++) {
      decoded |= BigInt(valueBytes[i]!) << BigInt(i * 8);
    }
    expect(decoded).toBe(0x0102030405060708090a0b0c0d0e0f10n);
  });

  it('defaults value to 0n', () => {
    const contract = new Uint8Array(32).fill(0);
    const payload1 = encodeContractCallPayload(
      contract,
      'x',
      new Uint8Array([]),
    );
    const payload2 = encodeContractCallPayload(
      contract,
      'x',
      new Uint8Array([]),
      0n,
    );
    expect(payload1).toEqual(payload2);
  });
});

// ---------------------------------------------------------------------------
// Phase 2 Serializers — Mining
// ---------------------------------------------------------------------------

describe('encodeMinerRegisterPayload', () => {
  it('produces correct layout', () => {
    const ipAddr = new Uint8Array([192, 168, 1, 1]);
    const name = 'miner1';
    const payload = encodeMinerRegisterPayload(2, ipAddr, name);
    // tier: 1, ipAddr: 4 (len) + 4 (data) = 8, name: 4 + 6 = 10
    // Total = 19
    expect(payload).toHaveLength(19);
  });

  it('encodes tier (u8) as first byte', () => {
    const ipAddr = new Uint8Array([10, 0, 0, 1]);
    const payload = encodeMinerRegisterPayload(5, ipAddr, 'node');
    expect(payload[0]).toBe(5);
  });

  it('encodes ipAddr as Vec<u8> with length prefix', () => {
    const ipAddr = new Uint8Array([8, 8, 8, 8]);
    const payload = encodeMinerRegisterPayload(1, ipAddr, 'x');
    // Offset 1: ipAddr length (4) as u32 LE
    expect(payload[1]).toBe(4);
    expect(payload[2]).toBe(0);
    expect(payload[3]).toBe(0);
    expect(payload[4]).toBe(0);
    // ipAddr bytes
    expect(payload[5]).toBe(8);
    expect(payload[6]).toBe(8);
    expect(payload[7]).toBe(8);
    expect(payload[8]).toBe(8);
  });

  it('encodes name as string after ipAddr', () => {
    const ipAddr = new Uint8Array([127, 0, 0, 1]);
    const name = 'local';
    const payload = encodeMinerRegisterPayload(0, ipAddr, name);
    // Offset 9: name length (5) as u32 LE
    expect(payload[9]).toBe(5);
    expect(payload[10]).toBe(0);
    expect(payload[11]).toBe(0);
    expect(payload[12]).toBe(0);
    // Name bytes
    expect(payload[13]).toBe(0x6c); // 'l'
    expect(payload[14]).toBe(0x6f); // 'o'
    expect(payload[15]).toBe(0x63); // 'c'
    expect(payload[16]).toBe(0x61); // 'a'
    expect(payload[17]).toBe(0x6c); // 'l'
  });

  it('handles single-byte IP', () => {
    const ipAddr = new Uint8Array([255]);
    const payload = encodeMinerRegisterPayload(7, ipAddr, 'a');
    // tier: 1, ipAddr len+data: 4+1=5, name len+data: 4+1=5. Total = 1+5+5=11
    expect(payload).toHaveLength(11);
  });
});

describe('encodeMinerHeartbeatPayload', () => {
  it('produces 40 bytes (32 + 8)', () => {
    const blockHash = new Uint8Array(32).fill(0xff);
    const payload = encodeMinerHeartbeatPayload(blockHash, 12345n);
    expect(payload).toHaveLength(40);
  });

  it('encodes blockHash in first 32 bytes', () => {
    const blockHash = new Uint8Array(32).fill(0x42);
    const payload = encodeMinerHeartbeatPayload(blockHash, 0n);
    expect(payload.slice(0, 32)).toEqual(blockHash);
  });

  it('encodes height (u64) in last 8 bytes as LE', () => {
    const blockHash = new Uint8Array(32).fill(0);
    const height = 0x0102030405060708n;
    const payload = encodeMinerHeartbeatPayload(blockHash, height);
    const heightBytes = payload.slice(32, 40);
    expect(heightBytes[0]).toBe(0x08);
    expect(heightBytes[1]).toBe(0x07);
    expect(heightBytes[2]).toBe(0x06);
    expect(heightBytes[3]).toBe(0x05);
    expect(heightBytes[4]).toBe(0x04);
    expect(heightBytes[5]).toBe(0x03);
    expect(heightBytes[6]).toBe(0x02);
    expect(heightBytes[7]).toBe(0x01);
  });

  it('decodes height correctly', () => {
    const blockHash = new Uint8Array(32).fill(0);
    const height = 9999999999n;
    const payload = encodeMinerHeartbeatPayload(blockHash, height);
    const heightBytes = payload.slice(32, 40);
    let decoded = 0n;
    for (let i = 0; i < 8; i++) {
      decoded |= BigInt(heightBytes[i]!) << BigInt(i * 8);
    }
    expect(decoded).toBe(height);
  });
});

// ---------------------------------------------------------------------------
// Client modules existence
// ---------------------------------------------------------------------------

describe('ClawClient modules', () => {
  it('has staking module with expected methods', () => {
    const client = new ClawClient();
    expect(client.staking).toBeDefined();
    expect(typeof client.staking.deposit).toBe('function');
    expect(typeof client.staking.withdraw).toBe('function');
    expect(typeof client.staking.claim).toBe('function');
    expect(typeof client.staking.changeDelegation).toBe('function');
  });

  it('has contract module with expected methods', () => {
    const client = new ClawClient();
    expect(client.contract).toBeDefined();
    expect(typeof client.contract.deploy).toBe('function');
    expect(typeof client.contract.call).toBe('function');
  });

  it('has miner module with expected methods', () => {
    const client = new ClawClient();
    expect(client.miner).toBeDefined();
    expect(typeof client.miner.register).toBe('function');
    expect(typeof client.miner.heartbeat).toBe('function');
  });

  it('has existing modules still available', () => {
    const client = new ClawClient();
    expect(client.agent).toBeDefined();
    expect(client.token).toBeDefined();
    expect(client.service).toBeDefined();
    expect(client.block).toBeDefined();
  });
});

// ---------------------------------------------------------------------------
// Client getTransaction
// ---------------------------------------------------------------------------

describe('ClawClient getTransaction', () => {
  it('calls correct RPC method', async () => {
    const client = new ClawClient();
    const mockCall = vi.spyOn(client['rpc'], 'call');

    try {
      await client.getTransaction('0xabcd');
    } catch {
      // Expected to fail with mock RPC
    }

    expect(mockCall).toHaveBeenCalledWith('claw_getTransactionByHash', [
      '0xabcd',
    ]);
  });

  it('passes txHash as first parameter', async () => {
    const client = new ClawClient();
    const mockCall = vi.spyOn(client['rpc'], 'call');

    try {
      await client.getTransaction('0x1234567890abcdef');
    } catch {
      // Expected to fail with mock RPC
    }

    expect(mockCall).toHaveBeenCalledWith(
      'claw_getTransactionByHash',
      ['0x1234567890abcdef'],
    );
  });
});
