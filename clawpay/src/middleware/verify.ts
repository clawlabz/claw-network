/**
 * Shared payment verification logic used by all middleware adapters.
 */

import { type PayCredential, CLAW_DECIMALS } from '../core/types.js';
import { type RpcClient } from '../core/rpc.js';
import { parseAmount } from '../core/transaction.js';

export interface VerifyResult {
  readonly valid: boolean;
  readonly reason?: string;
  readonly blockHeight?: number;
}

/**
 * Verify a payment credential against the chain.
 *
 * Checks:
 * 1. Transaction exists and is confirmed (has a receipt).
 * 2. Transaction details (recipient, amount) match the challenge requirements.
 */
export async function verifyPayment(
  rpc: RpcClient,
  credential: PayCredential,
  expectedRecipient: string,
  expectedAmount: string,
  token?: string,
): Promise<VerifyResult> {
  // 1. Check transaction receipt exists (means it's confirmed)
  const receipt = await rpc.getTransactionReceipt(credential.tx_hash);
  if (!receipt) {
    return { valid: false, reason: 'Transaction not confirmed' };
  }

  // 2. Get full transaction details to verify recipient and amount
  const txInfo = await rpc.getTransactionByHash(credential.tx_hash);
  if (!txInfo) {
    return { valid: false, reason: 'Transaction not found' };
  }

  // 3. Check it's a TokenTransfer
  if (txInfo.typeName !== 'TokenTransfer') {
    return { valid: false, reason: `Expected TokenTransfer, got ${txInfo.typeName}` };
  }

  // 4. Verify recipient
  if (txInfo.to?.toLowerCase() !== expectedRecipient.toLowerCase()) {
    return { valid: false, reason: 'Recipient mismatch' };
  }

  // 5. Verify amount (compare in base units)
  const requiredBaseUnits = parseAmount(expectedAmount, CLAW_DECIMALS);
  const actualBaseUnits = BigInt(txInfo.amount ?? '0');
  if (actualBaseUnits < requiredBaseUnits) {
    return { valid: false, reason: 'Insufficient payment amount' };
  }

  return { valid: true, blockHeight: receipt.blockHeight };
}
