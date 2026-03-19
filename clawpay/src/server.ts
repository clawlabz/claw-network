/**
 * ClawPay Server — payment-gated API middleware factory.
 *
 * Usage:
 *   import { ClawPay } from '@clawlabz/clawpay'
 *
 *   const pay = ClawPay.create({ privateKey: '...', rpc: '...' })
 *
 *   // Express
 *   app.post('/api/translate', pay.charge({ amount: '10' }), handler)
 *
 *   // Next.js
 *   export const POST = pay.protect({ amount: '10' }, async (req) => {
 *     return Response.json({ result: '...' })
 *   })
 */

import { type ClawPayConfig, type ChargeOptions, RPC_MAINNET } from './core/types.js';
import { Wallet } from './core/wallet.js';
import { RpcClient } from './core/rpc.js';
import { createExpressMiddleware, type ExpressMiddleware } from './middleware/express.js';
import { createNextjsHandler } from './middleware/nextjs.js';
import { createHonoMiddleware, type HonoMiddleware } from './middleware/hono.js';

export interface ClawPayServer {
  /** The wallet address receiving payments. */
  readonly address: string;

  /** Express middleware — use with app.use() or route-level middleware. */
  charge(options: ChargeOptions): ExpressMiddleware;

  /** Next.js Route Handler wrapper. */
  protect(
    options: ChargeOptions,
    handler: (req: Request) => Promise<Response> | Response,
  ): (req: Request) => Promise<Response> | Response;

  /** Hono middleware. */
  honoCharge(options: ChargeOptions): HonoMiddleware;
}

/**
 * Create a ClawPay server instance.
 */
export async function createServer(config: ClawPayConfig): Promise<ClawPayServer> {
  const wallet = await Wallet.fromPrivateKey(config.privateKey);
  const rpc = new RpcClient({
    url: config.rpc ?? RPC_MAINNET,
    timeout: config.timeout,
    maxRetries: config.maxRetries,
  });

  return {
    address: wallet.address,

    charge(options: ChargeOptions): ExpressMiddleware {
      return createExpressMiddleware(wallet, rpc, options);
    },

    protect(
      options: ChargeOptions,
      handler: (req: Request) => Promise<Response> | Response,
    ) {
      return createNextjsHandler(wallet, rpc, options, handler);
    },

    honoCharge(options: ChargeOptions): HonoMiddleware {
      return createHonoMiddleware(wallet, rpc, options);
    },
  };
}
