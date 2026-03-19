/**
 * Hono middleware for HTTP 402 payment gating.
 *
 * Usage:
 *   import { Hono } from 'hono'
 *   import { ClawPay } from '@clawlabz/clawpay'
 *
 *   const pay = ClawPay.create({ privateKey: '...', rpc: '...' })
 *   const app = new Hono()
 *   app.post('/api/translate', pay.charge({ amount: '10' }), (c) => {
 *     return c.json({ result: '...' })
 *   })
 */

import { type ChargeOptions } from '../core/types.js';
import { type RpcClient } from '../core/rpc.js';
import { type Wallet } from '../core/wallet.js';
import { createChallenge, HEADER_CLAW_PAY } from '../protocol/challenge.js';
import { parseCredential, HEADER_CLAW_CREDENTIAL } from '../protocol/credential.js';
import { createReceipt, serializeReceipt, HEADER_CLAW_RECEIPT } from '../protocol/receipt.js';
import { verifyPayment } from './verify.js';

// Minimal Hono-compatible types (avoids importing hono)
interface HonoContext {
  req: {
    header(name: string): string | undefined;
  };
  header(name: string, value: string): void;
  json(data: unknown, status?: number): Response;
}

type HonoNext = () => Promise<void>;

export type HonoMiddleware = (c: HonoContext, next: HonoNext) => Promise<Response | void>;

/**
 * Create a Hono middleware that gates a route behind CLAW payment.
 */
export function createHonoMiddleware(
  wallet: Wallet,
  rpc: RpcClient,
  options: ChargeOptions,
): HonoMiddleware {
  return async (c, next) => {
    const credentialHeader = c.req.header(HEADER_CLAW_CREDENTIAL);

    // No credential — return 402 with challenge
    if (!credentialHeader) {
      const challenge = createChallenge({
        recipient: wallet.address,
        amount: options.amount,
        token: options.token,
        expiresIn: options.expiresIn,
      });
      c.header(HEADER_CLAW_PAY, JSON.stringify(challenge));
      return c.json({ error: 'Payment Required', challenge }, 402);
    }

    // Has credential — verify payment on chain
    try {
      const credential = parseCredential(credentialHeader);
      const result = await verifyPayment(rpc, credential, wallet.address, options.amount, options.token);

      if (!result.valid) {
        return c.json({ error: 'Payment verification failed', reason: result.reason }, 402);
      }

      // Payment verified — add receipt header and continue
      const receipt = createReceipt(credential.tx_hash, result.blockHeight!);
      c.header(HEADER_CLAW_RECEIPT, serializeReceipt(receipt));
      await next();
    } catch (err) {
      return c.json(
        {
          error: 'Payment verification failed',
          reason: err instanceof Error ? err.message : 'Unknown error',
        },
        402,
      );
    }
  };
}
