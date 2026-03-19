import { describe, it, expect, vi, beforeEach } from 'vitest';
import { Wallet } from '../src/core/wallet.js';
import { RpcClient } from '../src/core/rpc.js';
import { createExpressMiddleware } from '../src/middleware/express.js';
import { createNextjsHandler } from '../src/middleware/nextjs.js';
import { createHonoMiddleware } from '../src/middleware/hono.js';
import { HEADER_CLAW_PAY } from '../src/protocol/challenge.js';
import { HEADER_CLAW_CREDENTIAL } from '../src/protocol/credential.js';
import { HEADER_CLAW_RECEIPT } from '../src/protocol/receipt.js';

// Mock RPC client
function createMockRpc(overrides: Partial<RpcClient> = {}): RpcClient {
  return {
    url: 'http://mock',
    timeout: 10_000,
    maxRetries: 3,
    call: vi.fn(),
    getBlockNumber: vi.fn(),
    getBalance: vi.fn(),
    getNonce: vi.fn(),
    sendTransaction: vi.fn(),
    getTransactionReceipt: vi.fn().mockResolvedValue({ blockHeight: 100, transactionIndex: 0 }),
    getTransactionByHash: vi.fn().mockResolvedValue(null),
    getAgent: vi.fn(),
    getServices: vi.fn(),
    getTokenBalance: vi.fn(),
    waitForConfirmation: vi.fn(),
    ...overrides,
  } as unknown as RpcClient;
}

describe('Express middleware', () => {
  let wallet: Wallet;

  beforeEach(async () => {
    wallet = await Wallet.generate();
  });

  it('should return 402 when no credential header', async () => {
    const rpc = createMockRpc();
    const middleware = createExpressMiddleware(wallet, rpc, { amount: '10' });

    const req = { headers: {} };
    const res = {
      status: vi.fn().mockReturnThis(),
      set: vi.fn().mockReturnThis(),
      json: vi.fn(),
    };
    const next = vi.fn();

    await middleware(req, res as any, next);

    expect(res.status).toHaveBeenCalledWith(402);
    expect(res.set).toHaveBeenCalledWith(
      HEADER_CLAW_PAY,
      expect.any(String),
    );
    expect(next).not.toHaveBeenCalled();
  });

  it('should call next() when payment is verified', async () => {
    const rpc = createMockRpc({
      getTransactionReceipt: vi.fn().mockResolvedValue({ blockHeight: 100, transactionIndex: 0 }),
      getTransactionByHash: vi.fn().mockResolvedValue({
        hash: 'abc',
        txType: 1,
        typeName: 'TokenTransfer',
        from: 'sender',
        to: wallet.address,
        amount: '10000000000', // 10 CLAW in base units
        nonce: 1,
        blockHeight: 100,
        timestamp: 1234567890,
        fee: '1000000',
      }),
    });

    const middleware = createExpressMiddleware(wallet, rpc, { amount: '10' });

    const credential = JSON.stringify({
      challenge_id: 'test-challenge',
      tx_hash: 'test-tx-hash',
    });

    const req = { headers: { [HEADER_CLAW_CREDENTIAL]: credential } };
    const res = {
      status: vi.fn().mockReturnThis(),
      set: vi.fn().mockReturnThis(),
      json: vi.fn(),
    };
    const next = vi.fn();

    await middleware(req, res as any, next);

    expect(next).toHaveBeenCalled();
    expect(res.set).toHaveBeenCalledWith(
      HEADER_CLAW_RECEIPT,
      expect.any(String),
    );
  });

  it('should return 402 when payment amount is insufficient', async () => {
    const rpc = createMockRpc({
      getTransactionReceipt: vi.fn().mockResolvedValue({ blockHeight: 100, transactionIndex: 0 }),
      getTransactionByHash: vi.fn().mockResolvedValue({
        hash: 'abc',
        txType: 1,
        typeName: 'TokenTransfer',
        from: 'sender',
        to: wallet.address,
        amount: '1000000000', // 1 CLAW (need 10)
        nonce: 1,
        blockHeight: 100,
        timestamp: 1234567890,
        fee: '1000000',
      }),
    });

    const middleware = createExpressMiddleware(wallet, rpc, { amount: '10' });

    const credential = JSON.stringify({
      challenge_id: 'test-challenge',
      tx_hash: 'test-tx-hash',
    });

    const req = { headers: { [HEADER_CLAW_CREDENTIAL]: credential } };
    const res = {
      status: vi.fn().mockReturnThis(),
      set: vi.fn().mockReturnThis(),
      json: vi.fn(),
    };
    const next = vi.fn();

    await middleware(req, res as any, next);

    expect(res.status).toHaveBeenCalledWith(402);
    expect(next).not.toHaveBeenCalled();
  });
});

describe('Next.js handler', () => {
  let wallet: Wallet;

  beforeEach(async () => {
    wallet = await Wallet.generate();
  });

  it('should return 402 when no credential', async () => {
    const rpc = createMockRpc();
    const handler = createNextjsHandler(
      wallet,
      rpc,
      { amount: '5' },
      async () => Response.json({ ok: true }),
    );

    const req = new Request('http://localhost/api/test', { method: 'POST' });
    const res = await handler(req);

    expect(res.status).toBe(402);
    expect(res.headers.get(HEADER_CLAW_PAY)).toBeTruthy();
  });

  it('should call handler and add receipt when payment is valid', async () => {
    const rpc = createMockRpc({
      getTransactionReceipt: vi.fn().mockResolvedValue({ blockHeight: 50, transactionIndex: 0 }),
      getTransactionByHash: vi.fn().mockResolvedValue({
        hash: 'abc',
        txType: 1,
        typeName: 'TokenTransfer',
        from: 'sender',
        to: wallet.address,
        amount: '5000000000',
        nonce: 1,
        blockHeight: 50,
        timestamp: 1234567890,
        fee: '1000000',
      }),
    });

    const handler = createNextjsHandler(
      wallet,
      rpc,
      { amount: '5' },
      async () => Response.json({ result: 'translated' }),
    );

    const credential = JSON.stringify({
      challenge_id: 'chal-1',
      tx_hash: 'tx-1',
    });

    const req = new Request('http://localhost/api/test', {
      method: 'POST',
      headers: { [HEADER_CLAW_CREDENTIAL]: credential },
    });

    const res = await handler(req);

    expect(res.status).toBe(200);
    expect(res.headers.get(HEADER_CLAW_RECEIPT)).toBeTruthy();
    const body = await res.json();
    expect(body.result).toBe('translated');
  });
});

describe('Hono middleware', () => {
  let wallet: Wallet;

  beforeEach(async () => {
    wallet = await Wallet.generate();
  });

  it('should return 402 when no credential', async () => {
    const rpc = createMockRpc();
    const middleware = createHonoMiddleware(wallet, rpc, { amount: '10' });

    const headers: Record<string, string> = {};
    const c = {
      req: { header: (name: string) => undefined },
      header: (name: string, value: string) => { headers[name] = value; },
      json: (data: unknown, status?: number) => {
        return new Response(JSON.stringify(data), { status: status ?? 200 });
      },
    };
    const next = vi.fn();

    const result = await middleware(c as any, next);

    expect(result).toBeInstanceOf(Response);
    expect((result as Response).status).toBe(402);
    expect(next).not.toHaveBeenCalled();
  });
});
