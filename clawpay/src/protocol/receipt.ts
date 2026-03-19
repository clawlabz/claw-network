/**
 * HTTP 402 Receipt — confirmation of settled payment.
 *
 * Attached as `X-Claw-Receipt` header in the successful response
 * after payment verification.
 */

import { type PayReceipt } from '../core/types.js';

/** Header name for the payment receipt. */
export const HEADER_CLAW_RECEIPT = 'x-claw-receipt';

/** Create a receipt object after verifying payment on-chain. */
export function createReceipt(
  txHash: string,
  blockHeight: number,
): PayReceipt {
  return {
    tx_hash: txHash,
    block_height: blockHeight,
    settled: true,
  };
}

/** Serialize a receipt to a header value string. */
export function serializeReceipt(receipt: PayReceipt): string {
  return JSON.stringify(receipt);
}

/** Parse a receipt from the X-Claw-Receipt header value. */
export function parseReceipt(headerValue: string): PayReceipt {
  try {
    const parsed = JSON.parse(headerValue) as PayReceipt;

    if (!parsed.tx_hash || typeof parsed.tx_hash !== 'string') {
      throw new Error('Missing or invalid tx_hash');
    }
    if (typeof parsed.block_height !== 'number') {
      throw new Error('Missing or invalid block_height');
    }

    return {
      tx_hash: parsed.tx_hash,
      block_height: parsed.block_height,
      settled: parsed.settled ?? true,
    };
  } catch (err) {
    if (err instanceof SyntaxError) {
      throw new Error('Invalid X-Claw-Receipt header: not valid JSON');
    }
    throw err;
  }
}
