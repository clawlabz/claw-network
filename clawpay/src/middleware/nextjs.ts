/**
 * Next.js App Router middleware for HTTP 402 payment gating.
 *
 * Usage:
 *   import { ClawPay } from '@clawlabz/clawpay'
 *   const pay = ClawPay.create({ privateKey: '...', rpc: '...' })
 *
 *   export const POST = pay.protect({ amount: '10' }, async (req) => {
 *     return Response.json({ result: '...' })
 *   })
 */

import { type ChargeOptions } from '../core/types.js';
import { type RpcClient } from '../core/rpc.js';
import { type Wallet } from '../core/wallet.js';
import { createChallenge, HEADER_CLAW_PAY } from '../protocol/challenge.js';
import { parseCredential, HEADER_CLAW_CREDENTIAL } from '../protocol/credential.js';
import { createReceipt, serializeReceipt, HEADER_CLAW_RECEIPT } from '../protocol/receipt.js';
import { verifyPayment } from './verify.js';

type NextHandler = (req: Request) => Promise<Response> | Response;

/**
 * Create a Next.js Route Handler wrapper that gates behind CLAW payment.
 * Returns a standard Web API Request/Response handler.
 */
export function createNextjsHandler(
  wallet: Wallet,
  rpc: RpcClient,
  options: ChargeOptions,
  handler: NextHandler,
): NextHandler {
  return async (req: Request): Promise<Response> => {
    const credentialHeader = req.headers.get(HEADER_CLAW_CREDENTIAL);

    // No credential — return 402 with challenge
    if (!credentialHeader) {
      const challenge = createChallenge({
        recipient: wallet.address,
        amount: options.amount,
        token: options.token,
        expiresIn: options.expiresIn,
      });

      return new Response(
        JSON.stringify({ error: 'Payment Required', challenge }),
        {
          status: 402,
          headers: {
            'Content-Type': 'application/json',
            [HEADER_CLAW_PAY]: JSON.stringify(challenge),
          },
        },
      );
    }

    // Has credential — verify payment on chain
    try {
      const credential = parseCredential(credentialHeader);
      const result = await verifyPayment(rpc, credential, wallet.address, options.amount, options.token);

      if (!result.valid) {
        return new Response(
          JSON.stringify({ error: 'Payment verification failed', reason: result.reason }),
          { status: 402, headers: { 'Content-Type': 'application/json' } },
        );
      }

      // Payment verified — call the actual handler
      const response = await handler(req);

      // Clone response to add receipt header
      const receipt = createReceipt(credential.tx_hash, result.blockHeight!);
      const newHeaders = new Headers(response.headers);
      newHeaders.set(HEADER_CLAW_RECEIPT, serializeReceipt(receipt));

      return new Response(response.body, {
        status: response.status,
        statusText: response.statusText,
        headers: newHeaders,
      });
    } catch (err) {
      return new Response(
        JSON.stringify({
          error: 'Payment verification failed',
          reason: err instanceof Error ? err.message : 'Unknown error',
        }),
        { status: 402, headers: { 'Content-Type': 'application/json' } },
      );
    }
  };
}
