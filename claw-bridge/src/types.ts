// ---------------------------------------------------------------------------
// CP↔CLAW Bridge types
// ---------------------------------------------------------------------------

export type BridgeDirection = 'cp_to_claw' | 'claw_to_cp';

export interface BridgeConfig {
  /** Supabase URL */
  supabaseUrl: string;
  /** Supabase service role key (server-side only) */
  supabaseServiceKey: string;
  /** ClawNetwork RPC URL (default: http://localhost:9710) */
  rpcUrl?: string;
  /** Hot wallet private key hex (bridge operator wallet) */
  hotWalletPrivateKey: string;
  /** CP per 1 CLAW base unit (default: 1 — 1:1 parity) */
  exchangeRate?: number;
  /** Max CP per agent per day (default: 100_000) */
  dailyLimitPerAgent?: number;
  /** Max CP globally per day (default: 10_000_000) */
  dailyLimitGlobal?: number;
}

export interface BridgeResult {
  success: boolean;
  direction: BridgeDirection;
  /** CP amount involved */
  cpAmount: number;
  /** CLAW base units involved (bigint as string) */
  clawAmount: string;
  /** Transaction hash on chain (for cp_to_claw) or burn confirmation */
  txHash?: string;
  /** Ledger entry ID in Supabase */
  ledgerId?: string;
  /** Error message if failed */
  error?: string;
}

export interface ExchangeRecord {
  id: string;
  agentId: string;
  direction: BridgeDirection;
  cpAmount: number;
  clawAmount: string;
  txHash: string | null;
  ledgerId: string | null;
  status: 'pending' | 'completed' | 'failed';
  createdAt: string;
}
