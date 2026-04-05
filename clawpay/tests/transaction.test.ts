import { describe, it, expect } from 'vitest';
import { Wallet, hexToBytes, bytesToHex } from '../src/core/wallet.js';
import {
  serializeTokenTransferPayload,
  buildSignableBytes,
  serializeTransaction,
  buildTransferTx,
  parseAmount,
  formatAmount,
} from '../src/core/transaction.js';
import { TxType, CLW_DECIMALS } from '../src/core/types.js';

describe('parseAmount / formatAmount', () => {
  it('should parse whole numbers', () => {
    expect(parseAmount('10', 9)).toBe(10_000_000_000n);
  });

  it('should parse decimal amounts', () => {
    expect(parseAmount('1.5', 9)).toBe(1_500_000_000n);
  });

  it('should parse zero', () => {
    expect(parseAmount('0', 9)).toBe(0n);
  });

  it('should truncate excess decimal places', () => {
    expect(parseAmount('1.1234567890', 9)).toBe(1_123_456_789n);
  });

  it('should pad short decimal places', () => {
    expect(parseAmount('1.1', 9)).toBe(1_100_000_000n);
  });

  it('should format whole amounts', () => {
    expect(formatAmount(10_000_000_000n, 9)).toBe('10');
  });

  it('should format fractional amounts', () => {
    expect(formatAmount(1_500_000_000n, 9)).toBe('1.5');
  });

  it('should format zero', () => {
    expect(formatAmount(0n, 9)).toBe('0');
  });

  it('should round-trip correctly', () => {
    const amounts = ['100', '0.001', '999.123456789', '0'];
    for (const amt of amounts) {
      const parsed = parseAmount(amt, 9);
      const formatted = formatAmount(parsed, 9);
      expect(formatted).toBe(amt);
    }
  });
});

describe('serializeTokenTransferPayload', () => {
  it('should produce 48 bytes (32 address + 16 u128)', () => {
    const to = new Uint8Array(32);
    to[0] = 0xaa;
    to[31] = 0xbb;

    const payload = serializeTokenTransferPayload({
      to,
      amount: 1000n,
    });

    expect(payload.length).toBe(48);

    // First 32 bytes = to address
    expect(payload[0]).toBe(0xaa);
    expect(payload[31]).toBe(0xbb);

    // Next 16 bytes = amount in LE u128
    // 1000 = 0x3E8 => [0xE8, 0x03, 0x00, ...]
    expect(payload[32]).toBe(0xe8);
    expect(payload[33]).toBe(0x03);
    expect(payload[34]).toBe(0x00);
  });
});

describe('buildSignableBytes', () => {
  it('should concatenate tx_type + from + nonce + payload', () => {
    const from = new Uint8Array(32).fill(0x01);
    const payload = new Uint8Array([0xaa, 0xbb]);
    const nonce = 5n;

    const signable = buildSignableBytes(TxType.TokenTransfer, from, nonce, payload);

    // 1 (tx_type) + 32 (from) + 8 (nonce) + 2 (payload) = 43
    expect(signable.length).toBe(43);

    // tx_type = 1 (TokenTransfer)
    expect(signable[0]).toBe(1);

    // from = all 0x01
    expect(signable[1]).toBe(0x01);
    expect(signable[32]).toBe(0x01);

    // nonce = 5 LE => [5, 0, 0, 0, 0, 0, 0, 0]
    expect(signable[33]).toBe(5);
    expect(signable[34]).toBe(0);

    // payload raw
    expect(signable[41]).toBe(0xaa);
    expect(signable[42]).toBe(0xbb);
  });
});

describe('serializeTransaction', () => {
  it('should produce correct Borsh bytes with length-prefixed payload', () => {
    const from = new Uint8Array(32).fill(0x02);
    const payload = new Uint8Array([0xff, 0xfe]);
    const signature = new Uint8Array(64).fill(0x03);

    const tx = {
      txType: TxType.TokenTransfer as TxType,
      from,
      nonce: 1n,
      payload,
      signature,
    };

    const serialized = serializeTransaction(tx);

    // 1 (type) + 32 (from) + 8 (nonce) + 4 (payload len) + 2 (payload) + 64 (sig) = 111
    expect(serialized.length).toBe(111);

    // tx_type
    expect(serialized[0]).toBe(1);

    // Payload length prefix (u32 LE) = 2
    expect(serialized[41]).toBe(2);
    expect(serialized[42]).toBe(0);
    expect(serialized[43]).toBe(0);
    expect(serialized[44]).toBe(0);

    // Payload data
    expect(serialized[45]).toBe(0xff);
    expect(serialized[46]).toBe(0xfe);

    // Signature starts at 47
    expect(serialized[47]).toBe(0x03);
    expect(serialized[110]).toBe(0x03);
  });
});

describe('buildTransferTx', () => {
  it('should build a valid signed transaction', async () => {
    const wallet = await Wallet.generate();
    const recipient = await Wallet.generate();

    const { tx, hash } = await buildTransferTx(wallet, 1n, {
      to: recipient.address,
      amount: '10',
    });

    expect(tx.txType).toBe(TxType.TokenTransfer);
    expect(bytesToHex(tx.from)).toBe(wallet.address);
    expect(tx.nonce).toBe(1n);
    expect(tx.signature.length).toBe(64);
    expect(hash.length).toBeGreaterThan(0);

    // Verify the signature
    const signable = buildSignableBytes(tx.txType, tx.from, tx.nonce, tx.payload);
    const valid = await wallet.verify(signable, tx.signature);
    expect(valid).toBe(true);
  });

  it('should reject invalid recipient address', async () => {
    const wallet = await Wallet.generate();
    await expect(
      buildTransferTx(wallet, 1n, { to: 'aabb', amount: '10' }),
    ).rejects.toThrow('32 bytes');
  });

  it('should reject zero amount', async () => {
    const wallet = await Wallet.generate();
    const recipient = await Wallet.generate();
    await expect(
      buildTransferTx(wallet, 1n, { to: recipient.address, amount: '0' }),
    ).rejects.toThrow('positive');
  });

  describe('Transfer regression (payload encoding + nonce + signature + receipt)', () => {
    it('should produce consistent payload encoding across multiple builds', async () => {
      const wallet = await Wallet.generate();
      const recipient = await Wallet.generate();

      // Build the same transfer twice
      const { tx: tx1 } = await buildTransferTx(wallet, 5n, {
        to: recipient.address,
        amount: '100.5',
      });
      const { tx: tx2 } = await buildTransferTx(wallet, 5n, {
        to: recipient.address,
        amount: '100.5',
      });

      // Payloads should be identical (same to + amount)
      expect(bytesToHex(tx1.payload)).toBe(bytesToHex(tx2.payload));
      expect(tx1.payload.length).toBe(48); // 32 bytes to + 16 bytes u128
    });

    it('should increment nonce correctly across sequential transfers', async () => {
      const wallet = await Wallet.generate();
      const recipient = await Wallet.generate();

      const { tx: tx1 } = await buildTransferTx(wallet, 10n, {
        to: recipient.address,
        amount: '5',
      });
      const { tx: tx2 } = await buildTransferTx(wallet, 11n, {
        to: recipient.address,
        amount: '5',
      });

      expect(tx1.nonce).toBe(10n);
      expect(tx2.nonce).toBe(11n);
    });

    it('should produce valid Ed25519 signatures', async () => {
      const wallet = await Wallet.generate();
      const recipient = await Wallet.generate();

      const { tx } = await buildTransferTx(wallet, 42n, {
        to: recipient.address,
        amount: '50.123',
      });

      // Rebuild signable bytes from tx components
      const signable = buildSignableBytes(tx.txType, tx.from, tx.nonce, tx.payload);

      // Verify signature is valid
      const isValid = await wallet.verify(signable, tx.signature);
      expect(isValid).toBe(true);

      // Signature must be 64 bytes
      expect(tx.signature.length).toBe(64);
    });

    it('should serialize transaction to consistent Borsh format', async () => {
      const wallet = await Wallet.generate();
      const recipient = await Wallet.generate();

      const { tx } = await buildTransferTx(wallet, 100n, {
        to: recipient.address,
        amount: '999',
      });

      const serialized = serializeTransaction(tx);

      // Verify structure: 1 (type) + 32 (from) + 8 (nonce) + 4 (payload_len) + 48 (payload) + 64 (sig) = 157 bytes
      expect(serialized.length).toBe(157);

      // Type byte at position 0
      expect(serialized[0]).toBe(TxType.TokenTransfer);

      // Sender address at positions 1-32
      expect(bytesToHex(serialized.slice(1, 33))).toBe(wallet.address);

      // Signature at the end (last 64 bytes)
      expect(bytesToHex(serialized.slice(93, 157))).toBe(bytesToHex(tx.signature));
    });

    it('should handle various decimal amounts correctly', async () => {
      const wallet = await Wallet.generate();
      const recipient = await Wallet.generate();

      const amounts = ['1', '0.1', '0.000000001', '999999.999999999'];

      for (const amount of amounts) {
        const { tx } = await buildTransferTx(wallet, 1n, {
          to: recipient.address,
          amount,
        });

        // Verify structure is preserved
        expect(tx.payload.length).toBe(48);
        expect(tx.signature.length).toBe(64);
        expect(tx.txType).toBe(TxType.TokenTransfer);

        // Verify signature is valid
        const signable = buildSignableBytes(tx.txType, tx.from, tx.nonce, tx.payload);
        const isValid = await wallet.verify(signable, tx.signature);
        expect(isValid).toBe(true);
      }
    });
  });
});
