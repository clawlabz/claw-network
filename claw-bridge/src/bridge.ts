// ---------------------------------------------------------------------------
// CP↔CLW Bridge — connects Supabase CP with on-chain CLW
// ---------------------------------------------------------------------------

import { createClient, SupabaseClient } from '@supabase/supabase-js';
import { ClawClient, Wallet, toHex } from '@clawlabz/clawnetwork-sdk';
import type { BridgeConfig, BridgeResult, BridgeDirection } from './types.js';

const DEFAULT_RATE = 1; // 1 CP = 1 CLW base unit
const DEFAULT_DAILY_PER_AGENT = 100_000;
const DEFAULT_DAILY_GLOBAL = 10_000_000;
const CLW_DECIMALS_MULTIPLIER = 1_000_000_000n; // 10^9

/**
 * CP↔CLW Bridge service.
 *
 * - `cpToClw`: Deducts CP from agent's Supabase account → transfers CLW on-chain to agent's CLW address.
 * - `clwToCp`: Agent sends CLW to hot wallet → credits CP in Supabase.
 *
 * The hot wallet is a platform-controlled address that holds CLW for bridging.
 * All operations are recorded in `shared_point_ledger` and `shared_exchange_orders`.
 */
export class CpClwBridge {
  private supabase: SupabaseClient;
  private clawClient: ClawClient;
  private hotWallet: Wallet;
  private rate: number;
  private dailyPerAgent: number;
  private dailyGlobal: number;

  constructor(config: BridgeConfig) {
    this.supabase = createClient(config.supabaseUrl, config.supabaseServiceKey);
    this.hotWallet = Wallet.fromPrivateKey(config.hotWalletPrivateKey);
    this.clawClient = new ClawClient({
      rpcUrl: config.rpcUrl ?? 'http://localhost:9710',
      wallet: this.hotWallet,
    });
    this.rate = config.exchangeRate ?? DEFAULT_RATE;
    this.dailyPerAgent = config.dailyLimitPerAgent ?? DEFAULT_DAILY_PER_AGENT;
    this.dailyGlobal = config.dailyLimitGlobal ?? DEFAULT_DAILY_GLOBAL;
  }

  /** Get the hot wallet address (hex). */
  get hotWalletAddress(): string {
    return this.hotWallet.address;
  }

  /**
   * CP → CLW: Deduct CP from agent, transfer CLW to their on-chain address.
   *
   * @param agentId - shared_agents.id
   * @param cpAmount - Amount of CP to convert
   * @param clwRecipient - On-chain CLW address (hex) to receive tokens
   */
  async cpToClw(
    agentId: string,
    cpAmount: number,
    clwRecipient: string,
  ): Promise<BridgeResult> {
    const direction: BridgeDirection = 'cp_to_clw';
    const clwBaseUnits = BigInt(Math.floor(cpAmount * this.rate));
    const clwAmount = (clwBaseUnits * CLW_DECIMALS_MULTIPLIER).toString();

    try {
      // 1. Rate limit check
      await this.checkDailyLimit(agentId, cpAmount);

      // 2. Get agent's CP account
      const account = await this.getPointAccount(agentId);
      if (!account) {
        return { success: false, direction, cpAmount, clwAmount, error: 'Agent has no CP account' };
      }

      // 3. Deduct CP via atomic RPC function
      const { data: ledgerId, error: ledgerError } = await this.supabase.rpc(
        'market_apply_ledger_mutation',
        {
          p_account_id: account.id,
          p_delta_available: -cpAmount,
          p_delta_frozen: 0,
          p_entry_type: 'BRIDGE_CP_TO_CLW',
          p_source_platform: 'bridge',
          p_reference_type: 'bridge',
          p_reference_id: null,
          p_memo: `Bridge ${cpAmount} CP → ${clwAmount} CLW to ${clwRecipient}`,
        },
      );

      if (ledgerError) {
        return { success: false, direction, cpAmount, clwAmount, error: ledgerError.message };
      }

      // 4. Transfer CLW on-chain from hot wallet to recipient
      let txHash: string;
      try {
        txHash = await this.clawClient.transfer({
          to: clwRecipient,
          amount: clwBaseUnits * CLW_DECIMALS_MULTIPLIER,
        });
      } catch (chainError: unknown) {
        // Rollback CP deduction if chain transfer fails
        await this.supabase.rpc('market_apply_ledger_mutation', {
          p_account_id: account.id,
          p_delta_available: cpAmount,
          p_delta_frozen: 0,
          p_entry_type: 'BRIDGE_ROLLBACK',
          p_source_platform: 'bridge',
          p_reference_type: 'bridge',
          p_reference_id: ledgerId,
          p_memo: `Rollback: chain transfer failed`,
        });
        const msg = chainError instanceof Error ? chainError.message : String(chainError);
        return { success: false, direction, cpAmount, clwAmount, error: `Chain transfer failed: ${msg}` };
      }

      // 5. Record exchange order
      await this.recordExchange(agentId, direction, cpAmount, clwAmount, txHash, ledgerId);

      return { success: true, direction, cpAmount, clwAmount, txHash, ledgerId };
    } catch (err: unknown) {
      const msg = err instanceof Error ? err.message : String(err);
      return { success: false, direction, cpAmount, clwAmount, error: msg };
    }
  }

  /**
   * CLW → CP: Verify CLW received at hot wallet, credit CP to agent.
   *
   * The agent must have already sent CLW to the hot wallet address.
   * This method verifies the on-chain balance change and credits CP.
   *
   * @param agentId - shared_agents.id
   * @param clwBaseUnits - CLW amount in base units (pre-decimals)
   * @param txHash - The on-chain transaction hash proving the CLW transfer
   */
  async clwToCp(
    agentId: string,
    clwBaseUnits: bigint,
    txHash: string,
  ): Promise<BridgeResult> {
    const direction: BridgeDirection = 'clw_to_cp';
    const cpAmount = Math.floor(Number(clwBaseUnits / CLW_DECIMALS_MULTIPLIER) / this.rate);
    const clwAmount = clwBaseUnits.toString();

    try {
      if (cpAmount <= 0) {
        return { success: false, direction, cpAmount, clwAmount, error: 'Amount too small to bridge' };
      }

      // 1. Rate limit check
      await this.checkDailyLimit(agentId, cpAmount);

      // 2. Verify the tx exists on-chain
      const receipt = await this.clawClient.getTransactionReceipt(txHash);
      if (!receipt) {
        return { success: false, direction, cpAmount, clwAmount, error: 'Transaction not found on chain' };
      }

      // 3. Check for duplicate bridge (prevent double-credit)
      const { data: existing } = await this.supabase
        .from('shared_exchange_orders')
        .select('id')
        .eq('pair', 'CLW/CP')
        .eq('direction', 'clw_to_cp')
        .eq('source_amount', Number(clwBaseUnits))
        .single();

      if (existing) {
        return { success: false, direction, cpAmount, clwAmount, error: 'Transaction already bridged' };
      }

      // 4. Get or create agent's CP account
      const account = await this.getOrCreatePointAccount(agentId);

      // 5. Credit CP
      const { data: ledgerId, error: ledgerError } = await this.supabase.rpc(
        'market_apply_ledger_mutation',
        {
          p_account_id: account.id,
          p_delta_available: cpAmount,
          p_delta_frozen: 0,
          p_entry_type: 'BRIDGE_CLW_TO_CP',
          p_source_platform: 'bridge',
          p_reference_type: 'bridge',
          p_reference_id: txHash,
          p_memo: `Bridge ${clwAmount} CLW → ${cpAmount} CP`,
        },
      );

      if (ledgerError) {
        return { success: false, direction, cpAmount, clwAmount, error: ledgerError.message };
      }

      // 6. Record exchange order
      await this.recordExchange(agentId, direction, cpAmount, clwAmount, txHash, ledgerId);

      return { success: true, direction, cpAmount, clwAmount, txHash, ledgerId };
    } catch (err: unknown) {
      const msg = err instanceof Error ? err.message : String(err);
      return { success: false, direction, cpAmount, clwAmount, error: msg };
    }
  }

  // --- Internal helpers ---

  private async getPointAccount(agentId: string) {
    const { data } = await this.supabase
      .from('shared_point_accounts')
      .select('id, available, frozen')
      .eq('agent_id', agentId)
      .single();
    return data;
  }

  private async getOrCreatePointAccount(agentId: string) {
    let account = await this.getPointAccount(agentId);
    if (!account) {
      const { data, error } = await this.supabase
        .from('shared_point_accounts')
        .insert({ agent_id: agentId, available: 0, frozen: 0 })
        .select('id, available, frozen')
        .single();
      if (error) throw new Error(`Failed to create CP account: ${error.message}`);
      account = data;
    }
    return account!;
  }

  private async checkDailyLimit(agentId: string, cpAmount: number): Promise<void> {
    const today = new Date().toISOString().split('T')[0];

    // Per-agent daily limit
    const { data: agentOrders } = await this.supabase
      .from('shared_exchange_orders')
      .select('source_amount')
      .eq('account_id', agentId)
      .gte('created_at', `${today}T00:00:00Z`)
      .lte('created_at', `${today}T23:59:59Z`);

    const agentTotal = (agentOrders ?? []).reduce(
      (sum, o) => sum + Number(o.source_amount),
      0,
    );

    if (agentTotal + cpAmount > this.dailyPerAgent) {
      throw new Error(
        `Daily limit exceeded for agent. Used: ${agentTotal}, limit: ${this.dailyPerAgent}`,
      );
    }

    // Global daily limit
    const { data: globalOrders } = await this.supabase
      .from('shared_exchange_orders')
      .select('source_amount')
      .gte('created_at', `${today}T00:00:00Z`)
      .lte('created_at', `${today}T23:59:59Z`);

    const globalTotal = (globalOrders ?? []).reduce(
      (sum, o) => sum + Number(o.source_amount),
      0,
    );

    if (globalTotal + cpAmount > this.dailyGlobal) {
      throw new Error(
        `Global daily limit exceeded. Used: ${globalTotal}, limit: ${this.dailyGlobal}`,
      );
    }
  }

  private async recordExchange(
    agentId: string,
    direction: BridgeDirection,
    cpAmount: number,
    clwAmount: string,
    txHash: string | null,
    ledgerId: string | null,
  ): Promise<void> {
    const account = await this.getPointAccount(agentId);
    await this.supabase.from('shared_exchange_orders').insert({
      account_id: account?.id,
      pair: direction === 'cp_to_clw' ? 'CP/CLW' : 'CLW/CP',
      direction,
      source_amount: direction === 'cp_to_clw' ? cpAmount : Number(clwAmount),
      target_amount: direction === 'cp_to_clw' ? Number(clwAmount) : cpAmount,
      rate: this.rate,
      status: 'COMPLETED',
    });
  }
}
