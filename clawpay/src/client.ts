/**
 * ClawPay Client — automatic HTTP 402 payment handling.
 *
 * Intercepts fetch globally so that any 402 response triggers automatic
 * on-chain payment and retry with credentials.
 *
 * Usage:
 *   import { ClawPay } from '@clawlabz/clawpay'
 *
 *   ClawPay.attach({ privateKey: process.env.AGENT_KEY })
 *
 *   // All subsequent fetch calls auto-handle 402
 *   const res = await fetch('https://agent.com/api/translate', {
 *     method: 'POST',
 *     body: JSON.stringify({ text: 'hello' }),
 *   })
 */

import { type ClawPayConfig, RPC_MAINNET } from './core/types.js';
import { Wallet } from './core/wallet.js';
import { RpcClient } from './core/rpc.js';
import { buildTransferTx } from './core/transaction.js';
import { bytesToHex } from './core/wallet.js';
import { parseChallenge, isChallengeExpired, HEADER_CLAW_PAY } from './protocol/challenge.js';
import { createCredential, serializeCredential, HEADER_CLAW_CREDENTIAL } from './protocol/credential.js';

/** Tracks whether we've already attached the global interceptor. */
let isAttached = false;
let originalFetch: typeof globalThis.fetch | undefined;

/**
 * Attach the ClawPay client to globalThis.fetch.
 * After calling this, all fetch requests that receive a 402 will
 * automatically pay on-chain and retry.
 */
export async function attachClient(config: ClawPayConfig): Promise<void> {
  if (isAttached) {
    throw new Error('ClawPay client is already attached. Call detach() first.');
  }

  const wallet = await Wallet.fromPrivateKey(config.privateKey);
  const rpc = new RpcClient({
    url: config.rpc ?? RPC_MAINNET,
    timeout: config.timeout,
    maxRetries: config.maxRetries,
  });

  originalFetch = globalThis.fetch;
  isAttached = true;

  const wrappedFetch: typeof globalThis.fetch = async (
    input: string | URL | Request,
    init?: RequestInit,
  ): Promise<Response> => {
    // Make the original request
    const response = await originalFetch!(input, init);

    // If not 402, pass through
    if (response.status !== 402) {
      return response;
    }

    // Check for X-Claw-Pay header
    const challengeHeader = response.headers.get(HEADER_CLAW_PAY);
    if (!challengeHeader) {
      // Not a ClawPay 402, return as-is
      return response;
    }

    // Parse the challenge
    const challenge = parseChallenge(challengeHeader);

    // Check expiry
    if (isChallengeExpired(challenge)) {
      throw new Error('Payment challenge has expired');
    }

    // Get nonce and build transfer transaction
    const nonce = await rpc.getNonce(wallet.address);

    const { hash: txHex } = await buildTransferTx(wallet, nonce + 1n, {
      to: challenge.recipient,
      amount: challenge.amount,
    });

    // Submit transaction
    const txHash = await rpc.sendTransaction(txHex);

    // Wait for confirmation (up to 15 seconds)
    await rpc.waitForConfirmation(txHash, 15_000, 1_000);

    // Build credential and retry original request
    const credential = createCredential(challenge.challenge_id, txHash);
    const retryHeaders = new Headers(init?.headers);
    retryHeaders.set(HEADER_CLAW_CREDENTIAL, serializeCredential(credential));

    return originalFetch!(input, {
      ...init,
      headers: retryHeaders,
    });
  };

  globalThis.fetch = wrappedFetch;
}

/**
 * Detach the ClawPay client and restore the original fetch.
 */
export function detachClient(): void {
  if (!isAttached || !originalFetch) {
    throw new Error('ClawPay client is not attached.');
  }

  globalThis.fetch = originalFetch;
  originalFetch = undefined;
  isAttached = false;
}

/**
 * Check if the ClawPay client is currently attached.
 */
export function isClientAttached(): boolean {
  return isAttached;
}
