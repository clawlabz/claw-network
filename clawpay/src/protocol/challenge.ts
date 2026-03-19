/**
 * HTTP 402 Challenge parsing and generation.
 *
 * The challenge is sent by the service provider in the `X-Claw-Pay` header
 * when a request lacks payment credentials.
 */

import { type PayChallenge } from '../core/types.js';

/** Header name for the payment challenge. */
export const HEADER_CLAW_PAY = 'x-claw-pay';

/** Generate a random challenge ID. */
export function generateChallengeId(): string {
  const bytes = new Uint8Array(16);
  crypto.getRandomValues(bytes);
  return Array.from(bytes)
    .map((b) => b.toString(16).padStart(2, '0'))
    .join('');
}

/** Create a PayChallenge object for a 402 response. */
export function createChallenge(options: {
  readonly recipient: string;
  readonly amount: string;
  readonly token?: string;
  readonly expiresIn?: number;
}): PayChallenge {
  const now = Math.floor(Date.now() / 1000);
  return {
    challenge_id: generateChallengeId(),
    recipient: options.recipient,
    amount: options.amount,
    token: options.token ?? 'CLAW',
    chain: 'clawnetwork',
    expires_at: now + (options.expiresIn ?? 300),
  };
}

/** Parse a PayChallenge from the X-Claw-Pay header value. */
export function parseChallenge(headerValue: string): PayChallenge {
  try {
    const parsed = JSON.parse(headerValue) as PayChallenge;

    if (!parsed.challenge_id || typeof parsed.challenge_id !== 'string') {
      throw new Error('Missing or invalid challenge_id');
    }
    if (!parsed.recipient || typeof parsed.recipient !== 'string') {
      throw new Error('Missing or invalid recipient');
    }
    if (!parsed.amount || typeof parsed.amount !== 'string') {
      throw new Error('Missing or invalid amount');
    }
    if (typeof parsed.expires_at !== 'number') {
      throw new Error('Missing or invalid expires_at');
    }

    return {
      challenge_id: parsed.challenge_id,
      recipient: parsed.recipient,
      amount: parsed.amount,
      token: parsed.token ?? 'CLAW',
      chain: parsed.chain ?? 'clawnetwork',
      expires_at: parsed.expires_at,
    };
  } catch (err) {
    if (err instanceof SyntaxError) {
      throw new Error(`Invalid X-Claw-Pay header: not valid JSON`);
    }
    throw err;
  }
}

/** Check if a challenge has expired. */
export function isChallengeExpired(challenge: PayChallenge): boolean {
  const now = Math.floor(Date.now() / 1000);
  return now >= challenge.expires_at;
}
