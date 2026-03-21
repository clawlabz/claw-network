/**
 * @clawlabz/clawpay — ClawNetwork payment SDK for AI Agents.
 *
 * Main entry point re-exporting all public APIs.
 */

// Core
export { Wallet, hexToBytes, bytesToHex } from './core/wallet.js';
export { RpcClient, RpcError } from './core/rpc.js';
export {
  buildTransferTx,
  buildServiceRegisterTx,
  serializeTransaction,
  serializeTokenTransferPayload,
  buildSignableBytes,
  parseAmount,
  formatAmount,
} from './core/transaction.js';
export {
  TxType,
  NATIVE_TOKEN_ID,
  CLAW_DECIMALS,
  CLW_DECIMALS,
  GAS_FEE,
  RPC_MAINNET,
  RPC_TESTNET,
} from './core/types.js';
export type {
  RawTransaction,
  TokenTransferPayload,
  ClawPayConfig,
  PayChallenge,
  PayCredential,
  PayReceipt,
  ChargeOptions,
  TransactionReceipt,
  TransactionInfo,
  AgentIdentity,
  ServiceEntry,
} from './core/types.js';

// Protocol
export {
  createChallenge,
  parseChallenge,
  isChallengeExpired,
  generateChallengeId,
  HEADER_CLAW_PAY,
} from './protocol/challenge.js';
export {
  createCredential,
  parseCredential,
  serializeCredential,
  HEADER_CLAW_CREDENTIAL,
} from './protocol/credential.js';
export {
  createReceipt,
  parseReceipt,
  serializeReceipt,
  HEADER_CLAW_RECEIPT,
} from './protocol/receipt.js';

// Client
export { attachClient, detachClient, isClientAttached } from './client.js';

// Server
export { createServer } from './server.js';
export type { ClawPayServer } from './server.js';

// Middleware (direct access)
export { createExpressMiddleware } from './middleware/express.js';
export { createNextjsHandler } from './middleware/nextjs.js';
export { createHonoMiddleware } from './middleware/hono.js';
export { verifyPayment } from './middleware/verify.js';

// ---------------------------------------------------------------------------
// ClawPay namespace — convenience static methods
// ---------------------------------------------------------------------------

import { type ClawPayConfig } from './core/types.js';
import { attachClient, detachClient } from './client.js';
import { createServer, type ClawPayServer } from './server.js';

export const ClawPay = {
  /**
   * Create a ClawPay server instance for receiving payments.
   *
   * @example
   *   const pay = await ClawPay.create({ privateKey: '...', rpc: '...' })
   *   app.post('/api/translate', pay.charge({ amount: '10' }), handler)
   */
  create: createServer,

  /**
   * Attach automatic payment handling to global fetch.
   *
   * @example
   *   await ClawPay.attach({ privateKey: '...', rpc: '...' })
   *   const res = await fetch('https://agent.com/api/translate', { ... })
   */
  attach: attachClient,

  /**
   * Detach the client and restore original fetch.
   */
  detach: detachClient,
} as const;
