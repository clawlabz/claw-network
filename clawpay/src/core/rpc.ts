/**
 * ClawNetwork JSON-RPC 2.0 client.
 * Zero external dependencies — uses native fetch.
 */

import {
  type TransactionReceipt,
  type TransactionInfo,
  type AgentIdentity,
  type ServiceEntry,
  RPC_MAINNET,
} from './types.js';

// ---------------------------------------------------------------------------
// RPC client
// ---------------------------------------------------------------------------

export interface RpcClientOptions {
  /** RPC endpoint URL. */
  readonly url: string;
  /** Request timeout in milliseconds. Defaults to 10000. */
  readonly timeout?: number;
  /** Maximum retries on transient errors. Defaults to 3. */
  readonly maxRetries?: number;
}

interface JsonRpcResponse<T> {
  readonly jsonrpc: string;
  readonly id: number;
  readonly result?: T;
  readonly error?: { code: number; message: string };
}

let rpcIdCounter = 1;

export class RpcClient {
  private readonly url: string;
  private readonly timeout: number;
  private readonly maxRetries: number;

  constructor(options: RpcClientOptions) {
    this.url = options.url;
    this.timeout = options.timeout ?? 10_000;
    this.maxRetries = options.maxRetries ?? 3;
  }

  static mainnet(): RpcClient {
    return new RpcClient({ url: RPC_MAINNET });
  }

  /** Low-level JSON-RPC call with retry logic. */
  async call<T>(method: string, params: unknown[] = []): Promise<T> {
    const id = rpcIdCounter++;
    const body = JSON.stringify({
      jsonrpc: '2.0',
      id,
      method,
      params,
    });

    let lastError: Error | undefined;

    for (let attempt = 0; attempt <= this.maxRetries; attempt++) {
      try {
        const controller = new AbortController();
        const timer = setTimeout(() => controller.abort(), this.timeout);

        const response = await fetch(this.url, {
          method: 'POST',
          headers: { 'Content-Type': 'application/json' },
          body,
          signal: controller.signal,
        });

        clearTimeout(timer);

        if (!response.ok) {
          throw new Error(`RPC HTTP error: ${response.status} ${response.statusText}`);
        }

        const json = (await response.json()) as JsonRpcResponse<T>;

        if (json.error) {
          throw new RpcError(json.error.code, json.error.message);
        }

        return json.result as T;
      } catch (err) {
        lastError = err instanceof Error ? err : new Error(String(err));
        // Don't retry on RPC logic errors (non-transient)
        if (err instanceof RpcError) throw err;
        // Don't retry on last attempt
        if (attempt === this.maxRetries) break;
        // Brief backoff
        await sleep(100 * (attempt + 1));
      }
    }

    throw lastError ?? new Error('RPC call failed');
  }

  // -----------------------------------------------------------------------
  // High-level RPC methods
  // -----------------------------------------------------------------------

  /** Get the current block height. */
  async getBlockNumber(): Promise<number> {
    return this.call<number>('claw_blockNumber');
  }

  /** Get the native CLAW balance for an address (returned as string of base units). */
  async getBalance(address: string): Promise<bigint> {
    const result = await this.call<string>('claw_getBalance', [address]);
    return BigInt(result);
  }

  /** Get the current nonce for an address. */
  async getNonce(address: string): Promise<bigint> {
    const result = await this.call<number>('claw_getNonce', [address]);
    return BigInt(result);
  }

  /** Submit a signed transaction (hex-encoded Borsh bytes). Returns tx hash hex. */
  async sendTransaction(txHex: string): Promise<string> {
    return this.call<string>('claw_sendTransaction', [txHex]);
  }

  /** Get a transaction receipt by hash. Returns null if not found. */
  async getTransactionReceipt(txHash: string): Promise<TransactionReceipt | null> {
    return this.call<TransactionReceipt | null>('claw_getTransactionReceipt', [txHash]);
  }

  /** Get transaction details by hash. Returns null if not found. */
  async getTransactionByHash(txHash: string): Promise<TransactionInfo | null> {
    return this.call<TransactionInfo | null>('claw_getTransactionByHash', [txHash]);
  }

  /** Get registered agent identity. Returns null if not registered. */
  async getAgent(address: string): Promise<AgentIdentity | null> {
    return this.call<AgentIdentity | null>('claw_getAgent', [address]);
  }

  /** Get registered services, optionally filtered by type. */
  async getServices(serviceType?: string): Promise<ServiceEntry[]> {
    const params = serviceType ? [serviceType] : [];
    return this.call<ServiceEntry[]>('claw_getServices', params);
  }

  /** Get token balance for a specific token. */
  async getTokenBalance(address: string, tokenId: string): Promise<bigint> {
    const result = await this.call<string>('claw_getTokenBalance', [address, tokenId]);
    return BigInt(result);
  }

  /**
   * Wait for a transaction to be confirmed.
   * Polls getTransactionReceipt until found or timeout.
   */
  async waitForConfirmation(
    txHash: string,
    timeoutMs: number = 15_000,
    pollIntervalMs: number = 1_000,
  ): Promise<TransactionReceipt> {
    const deadline = Date.now() + timeoutMs;

    while (Date.now() < deadline) {
      const receipt = await this.getTransactionReceipt(txHash);
      if (receipt !== null) return receipt;
      await sleep(pollIntervalMs);
    }

    throw new Error(`Transaction ${txHash} not confirmed within ${timeoutMs}ms`);
  }
}

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

export class RpcError extends Error {
  readonly code: number;

  constructor(code: number, message: string) {
    super(`RPC error ${code}: ${message}`);
    this.name = 'RpcError';
    this.code = code;
  }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

function sleep(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms));
}
