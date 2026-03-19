/**
 * Express.js middleware for HTTP 402 payment gating.
 *
 * Usage:
 *   import { ClawPay } from '@clawlabz/clawpay'
 *   const pay = ClawPay.create({ privateKey: '...', rpc: '...' })
 *   app.post('/api/translate', pay.charge({ amount: '10' }), handler)
 */

import { type ChargeOptions } from '../core/types.js';
import { type RpcClient } from '../core/rpc.js';
import { type Wallet } from '../core/wallet.js';
import { createChallenge, HEADER_CLAW_PAY } from '../protocol/challenge.js';
import { parseCredential, HEADER_CLAW_CREDENTIAL } from '../protocol/credential.js';
import { createReceipt, serializeReceipt, HEADER_CLAW_RECEIPT } from '../protocol/receipt.js';
import { verifyPayment } from './verify.js';

// Minimal Express-compatible types (avoids importing express)
interface ExpressRequest {
  headers: Record<string, string | string[] | undefined>;
}

interface ExpressResponse {
  status(code: number): ExpressResponse;
  set(name: string, value: string): ExpressResponse;
  json(body: unknown): void;
}

type NextFunction = (err?: unknown) => void;

export type ExpressMiddleware = (
  req: ExpressRequest,
  res: ExpressResponse,
  next: NextFunction,
) => void;

/**
 * Create an Express middleware that gates a route behind CLAW payment.
 */
export function createExpressMiddleware(
  wallet: Wallet,
  rpc: RpcClient,
  options: ChargeOptions,
): ExpressMiddleware {
  return async (req, res, next) => {
    const credentialHeader = getHeader(req.headers, HEADER_CLAW_CREDENTIAL);

    // No credential — return 402 with challenge
    if (!credentialHeader) {
      const challenge = createChallenge({
        recipient: wallet.address,
        amount: options.amount,
        token: options.token,
        expiresIn: options.expiresIn,
      });
      res
        .status(402)
        .set(HEADER_CLAW_PAY, JSON.stringify(challenge))
        .json({
          error: 'Payment Required',
          challenge,
        });
      return;
    }

    // Has credential — verify payment on chain
    try {
      const credential = parseCredential(credentialHeader);
      const result = await verifyPayment(rpc, credential, wallet.address, options.amount, options.token);

      if (!result.valid) {
        res
          .status(402)
          .json({ error: 'Payment verification failed', reason: result.reason });
        return;
      }

      // Payment verified — add receipt header and continue
      const receipt = createReceipt(credential.tx_hash, result.blockHeight!);
      res.set(HEADER_CLAW_RECEIPT, serializeReceipt(receipt));
      next();
    } catch (err) {
      res
        .status(402)
        .json({
          error: 'Payment verification failed',
          reason: err instanceof Error ? err.message : 'Unknown error',
        });
    }
  };
}

function getHeader(
  headers: Record<string, string | string[] | undefined>,
  name: string,
): string | undefined {
  const value = headers[name] ?? headers[name.toLowerCase()];
  if (Array.isArray(value)) return value[0];
  return value;
}
