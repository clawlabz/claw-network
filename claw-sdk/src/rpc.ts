// ---------------------------------------------------------------------------
// ClawNetwork SDK — JSON-RPC 2.0 client (zero-dependency, uses native fetch)
// ---------------------------------------------------------------------------

export const DEFAULT_RPC_URL = 'http://localhost:9710';

export interface JsonRpcRequest {
  jsonrpc: '2.0';
  id: number;
  method: string;
  params: unknown[];
}

export interface JsonRpcResponse<T = unknown> {
  jsonrpc: '2.0';
  id: number;
  result?: T;
  error?: { code: number; message: string };
}

export class RpcError extends Error {
  constructor(
    public code: number,
    message: string,
  ) {
    super(message);
    this.name = 'RpcError';
  }
}

/**
 * Low-level JSON-RPC 2.0 client. Uses native `fetch` — no dependencies.
 */
export class RpcClient {
  private url: string;
  private nextId = 1;

  constructor(url: string = DEFAULT_RPC_URL) {
    this.url = url;
  }

  /**
   * Send a JSON-RPC 2.0 request and return the result.
   */
  async call<T = unknown>(method: string, params: unknown[] = []): Promise<T> {
    const body: JsonRpcRequest = {
      jsonrpc: '2.0',
      id: this.nextId++,
      method,
      params,
    };

    const response = await fetch(this.url, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify(body),
    });

    if (!response.ok) {
      throw new RpcError(
        -32000,
        `HTTP ${response.status}: ${response.statusText}`,
      );
    }

    const json = (await response.json()) as JsonRpcResponse<T>;

    if (json.error) {
      throw new RpcError(json.error.code, json.error.message);
    }

    return json.result as T;
  }
}
