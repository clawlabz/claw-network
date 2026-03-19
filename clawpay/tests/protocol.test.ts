import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import {
  createChallenge,
  parseChallenge,
  isChallengeExpired,
  generateChallengeId,
} from '../src/protocol/challenge.js';
import {
  createCredential,
  parseCredential,
  serializeCredential,
} from '../src/protocol/credential.js';
import {
  createReceipt,
  parseReceipt,
  serializeReceipt,
} from '../src/protocol/receipt.js';

describe('Challenge', () => {
  it('should create a valid challenge', () => {
    const challenge = createChallenge({
      recipient: 'aabb'.repeat(16),
      amount: '10',
      token: 'CLAW',
      expiresIn: 300,
    });

    expect(challenge.challenge_id.length).toBe(32); // 16 bytes hex
    expect(challenge.recipient).toBe('aabb'.repeat(16));
    expect(challenge.amount).toBe('10');
    expect(challenge.token).toBe('CLAW');
    expect(challenge.chain).toBe('clawnetwork');
    expect(challenge.expires_at).toBeGreaterThan(0);
  });

  it('should default token to CLAW', () => {
    const challenge = createChallenge({
      recipient: 'aa'.repeat(32),
      amount: '5',
    });
    expect(challenge.token).toBe('CLAW');
  });

  it('should generate unique challenge IDs', () => {
    const id1 = generateChallengeId();
    const id2 = generateChallengeId();
    expect(id1).not.toBe(id2);
  });

  it('should parse a valid challenge JSON', () => {
    const original = createChallenge({
      recipient: 'cc'.repeat(32),
      amount: '20',
    });
    const parsed = parseChallenge(JSON.stringify(original));
    expect(parsed.challenge_id).toBe(original.challenge_id);
    expect(parsed.recipient).toBe(original.recipient);
    expect(parsed.amount).toBe(original.amount);
  });

  it('should reject invalid JSON', () => {
    expect(() => parseChallenge('not json')).toThrow('not valid JSON');
  });

  it('should reject missing fields', () => {
    expect(() => parseChallenge('{}')).toThrow('Missing or invalid challenge_id');
  });

  it('should detect expired challenges', () => {
    const challenge = createChallenge({
      recipient: 'aa'.repeat(32),
      amount: '10',
      expiresIn: -1, // already expired
    });
    expect(isChallengeExpired(challenge)).toBe(true);
  });

  it('should detect non-expired challenges', () => {
    const challenge = createChallenge({
      recipient: 'aa'.repeat(32),
      amount: '10',
      expiresIn: 3600,
    });
    expect(isChallengeExpired(challenge)).toBe(false);
  });
});

describe('Credential', () => {
  it('should create and serialize a credential', () => {
    const credential = createCredential('challenge-123', 'txhash-456');
    expect(credential.challenge_id).toBe('challenge-123');
    expect(credential.tx_hash).toBe('txhash-456');

    const serialized = serializeCredential(credential);
    const parsed = parseCredential(serialized);
    expect(parsed.challenge_id).toBe('challenge-123');
    expect(parsed.tx_hash).toBe('txhash-456');
  });

  it('should reject invalid JSON', () => {
    expect(() => parseCredential('bad')).toThrow('not valid JSON');
  });

  it('should reject missing challenge_id', () => {
    expect(() => parseCredential('{"tx_hash":"abc"}')).toThrow('challenge_id');
  });

  it('should reject missing tx_hash', () => {
    expect(() => parseCredential('{"challenge_id":"abc"}')).toThrow('tx_hash');
  });
});

describe('Receipt', () => {
  it('should create and serialize a receipt', () => {
    const receipt = createReceipt('tx-hash-789', 42);
    expect(receipt.tx_hash).toBe('tx-hash-789');
    expect(receipt.block_height).toBe(42);
    expect(receipt.settled).toBe(true);

    const serialized = serializeReceipt(receipt);
    const parsed = parseReceipt(serialized);
    expect(parsed.tx_hash).toBe('tx-hash-789');
    expect(parsed.block_height).toBe(42);
    expect(parsed.settled).toBe(true);
  });

  it('should reject invalid JSON', () => {
    expect(() => parseReceipt('bad')).toThrow('not valid JSON');
  });

  it('should reject missing tx_hash', () => {
    expect(() => parseReceipt('{"block_height":1}')).toThrow('tx_hash');
  });

  it('should reject missing block_height', () => {
    expect(() => parseReceipt('{"tx_hash":"abc"}')).toThrow('block_height');
  });
});
