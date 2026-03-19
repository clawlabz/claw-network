/**
 * HTTP 402 Credential — proof of payment sent by the client.
 *
 * Attached as `X-Claw-Credential` header containing JSON with
 * challenge_id and tx_hash.
 */

import { type PayCredential } from '../core/types.js';

/** Header name for the payment credential. */
export const HEADER_CLAW_CREDENTIAL = 'x-claw-credential';

/** Create a credential object after completing payment. */
export function createCredential(
  challengeId: string,
  txHash: string,
): PayCredential {
  return {
    challenge_id: challengeId,
    tx_hash: txHash,
  };
}

/** Serialize a credential to a header value string. */
export function serializeCredential(credential: PayCredential): string {
  return JSON.stringify(credential);
}

/** Parse a credential from the X-Claw-Credential header value. */
export function parseCredential(headerValue: string): PayCredential {
  try {
    const parsed = JSON.parse(headerValue) as PayCredential;

    if (!parsed.challenge_id || typeof parsed.challenge_id !== 'string') {
      throw new Error('Missing or invalid challenge_id');
    }
    if (!parsed.tx_hash || typeof parsed.tx_hash !== 'string') {
      throw new Error('Missing or invalid tx_hash');
    }

    return {
      challenge_id: parsed.challenge_id,
      tx_hash: parsed.tx_hash,
    };
  } catch (err) {
    if (err instanceof SyntaxError) {
      throw new Error('Invalid X-Claw-Credential header: not valid JSON');
    }
    throw err;
  }
}
