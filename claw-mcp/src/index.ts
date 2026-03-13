#!/usr/bin/env node

import { McpServer } from '@modelcontextprotocol/sdk/server/mcp.js';
import { StdioServerTransport } from '@modelcontextprotocol/sdk/server/stdio.js';
import { ClawClient, Wallet, toHex } from '@clawlabz/clawnetwork-sdk';
import { z } from 'zod';
import { readFileSync, writeFileSync, mkdirSync, existsSync } from 'node:fs';
import { join } from 'node:path';
import { homedir } from 'node:os';

// ---------------------------------------------------------------------------
// Configuration
// ---------------------------------------------------------------------------

const RPC_URL = process.env.CLAW_RPC_URL ?? 'http://localhost:9710';
const WALLET_DIR = join(homedir(), '.claw-node');
const WALLET_PATH = join(WALLET_DIR, 'wallet.json');

// ---------------------------------------------------------------------------
// Wallet helpers
// ---------------------------------------------------------------------------

function loadOrCreateWallet(): Wallet {
  if (existsSync(WALLET_PATH)) {
    const raw = JSON.parse(readFileSync(WALLET_PATH, 'utf-8'));
    return Wallet.fromPrivateKey(raw.secret_key as string);
  }
  mkdirSync(WALLET_DIR, { recursive: true });
  const wallet = Wallet.generate();
  const data = JSON.stringify({
    address: wallet.address,
    secret_key: toHex(wallet.privateKey),
  }, null, 2);
  writeFileSync(WALLET_PATH, data, { mode: 0o600 });
  return wallet;
}

// ---------------------------------------------------------------------------
// Client initialisation
// ---------------------------------------------------------------------------

const wallet = loadOrCreateWallet();
const client = new ClawClient({ rpcUrl: RPC_URL, wallet });

// ---------------------------------------------------------------------------
// MCP Server
// ---------------------------------------------------------------------------

const server = new McpServer({
  name: 'claw-network',
  version: '0.1.0',
});

// Helper: wrap handler so errors become text content instead of throwing
function safe<T>(fn: () => Promise<T>): Promise<{ content: { type: 'text'; text: string }[] }> {
  return fn()
    .then((result) => ({
      content: [{ type: 'text' as const, text: JSON.stringify(result, (_k, v) => typeof v === 'bigint' ? v.toString() : v, 2) }],
    }))
    .catch((err: unknown) => ({
      content: [
        {
          type: 'text' as const,
          text: `Error: ${err instanceof Error ? err.message : String(err)}`,
        },
      ],
    }));
}

// ---------------------------------------------------------------------------
// 1. claw_status — Get node status
// ---------------------------------------------------------------------------

server.tool(
  'claw_status',
  'Get ClawNetwork node status (block height)',
  {},
  async () =>
    safe(async () => {
      const blockNumber = await client.block.getLatest();
      return { blockNumber, rpcUrl: RPC_URL, walletAddress: wallet.address };
    }),
);

// ---------------------------------------------------------------------------
// 2. claw_balance — Query CLW balance
// ---------------------------------------------------------------------------

server.tool(
  'claw_balance',
  'Query CLW balance for an address',
  { address: z.string().describe('Hex address to query') },
  async ({ address }) =>
    safe(async () => {
      const balance = await client.getBalance(address);
      return { address, balance: balance.toString() };
    }),
);

// ---------------------------------------------------------------------------
// 3. claw_transfer — Transfer CLW
// ---------------------------------------------------------------------------

server.tool(
  'claw_transfer',
  'Transfer CLW to another address',
  {
    to: z.string().describe('Recipient hex address'),
    amount: z.string().describe('Amount to transfer in base units (string for bigint)'),
  },
  async ({ to, amount }) =>
    safe(async () => {
      const txHash = await client.transfer({ to, amount: BigInt(amount) });
      return { txHash, to, amount };
    }),
);

// ---------------------------------------------------------------------------
// 4. claw_agent_register — Register an AI agent on-chain
// ---------------------------------------------------------------------------

server.tool(
  'claw_agent_register',
  'Register an AI agent on the ClawNetwork blockchain',
  {
    name: z.string().describe('Agent display name'),
    metadata: z.record(z.string()).optional().describe('Optional key-value metadata'),
  },
  async ({ name, metadata }) =>
    safe(async () => {
      const txHash = await client.agent.register({ name, metadata: metadata ?? {} });
      return { txHash, name };
    }),
);

// ---------------------------------------------------------------------------
// 5. claw_token_create — Create a custom token
// ---------------------------------------------------------------------------

server.tool(
  'claw_token_create',
  'Create a custom token on ClawNetwork',
  {
    name: z.string().describe('Token name'),
    symbol: z.string().describe('Token symbol'),
    decimals: z.number().describe('Decimal places'),
    totalSupply: z.string().describe('Total supply in base units (string for bigint)'),
  },
  async ({ name, symbol, decimals, totalSupply }) =>
    safe(async () => {
      const txHash = await client.token.create({
        name,
        symbol,
        decimals,
        totalSupply: BigInt(totalSupply),
      });
      return { txHash, name, symbol };
    }),
);

// ---------------------------------------------------------------------------
// 6. claw_token_transfer — Transfer custom token
// ---------------------------------------------------------------------------

server.tool(
  'claw_token_transfer',
  'Transfer a custom token to another address',
  {
    tokenId: z.string().describe('Token identifier hex'),
    to: z.string().describe('Recipient hex address'),
    amount: z.string().describe('Amount to transfer (string for bigint)'),
  },
  async ({ tokenId, to, amount }) =>
    safe(async () => {
      const txHash = await client.token.transfer({
        tokenId,
        to,
        amount: BigInt(amount),
      });
      return { txHash, tokenId, to, amount };
    }),
);

// ---------------------------------------------------------------------------
// 7. claw_reputation_attest — Write reputation attestation
// ---------------------------------------------------------------------------

server.tool(
  'claw_reputation_attest',
  'Write a reputation attestation for an address',
  {
    to: z.string().describe('Target address hex'),
    category: z.string().describe('Reputation category (e.g. "game", "task")'),
    score: z.number().describe('Score from -100 to 100'),
    platform: z.string().describe('Originating platform identifier'),
    memo: z.string().optional().describe('Optional memo'),
  },
  async ({ to, category, score, platform, memo }) =>
    safe(async () => {
      const txHash = await client.reputation.attest({
        to,
        category,
        score,
        platform,
        memo: memo ?? '',
      });
      return { txHash, to, category, score };
    }),
);

// ---------------------------------------------------------------------------
// 8. claw_reputation_get — Query reputation
// ---------------------------------------------------------------------------

server.tool(
  'claw_reputation_get',
  'Query reputation data for an address',
  { address: z.string().describe('Hex address to query') },
  async ({ address }) =>
    safe(async () => {
      const result = await client.reputation.get(address);
      return result;
    }),
);

// ---------------------------------------------------------------------------
// 9. claw_service_register — Register a service
// ---------------------------------------------------------------------------

server.tool(
  'claw_service_register',
  'Register a service on ClawNetwork',
  {
    serviceType: z.string().describe('Service type identifier'),
    description: z.string().describe('Service description'),
    priceToken: z.string().optional().describe('Price token hex (default: CLW = 00..00)'),
    priceAmount: z.string().describe('Price amount in base units (string for bigint)'),
    endpoint: z.string().describe('Service endpoint URL'),
    active: z.boolean().optional().describe('Whether service is active (default: true)'),
  },
  async ({ serviceType, description, priceToken, priceAmount, endpoint, active }) =>
    safe(async () => {
      const txHash = await client.service.register({
        serviceType,
        description,
        priceToken: priceToken ?? '00'.repeat(32),
        priceAmount: BigInt(priceAmount),
        endpoint,
        active: active ?? true,
      });
      return { txHash, serviceType, endpoint };
    }),
);

// ---------------------------------------------------------------------------
// 10. claw_service_search — Search services
// ---------------------------------------------------------------------------

server.tool(
  'claw_service_search',
  'Search for services on ClawNetwork',
  {
    serviceType: z.string().optional().describe('Filter by service type'),
  },
  async ({ serviceType }) =>
    safe(async () => {
      const result = await client.service.search({ serviceType });
      return result;
    }),
);

// ---------------------------------------------------------------------------
// Start server
// ---------------------------------------------------------------------------

async function main() {
  const transport = new StdioServerTransport();
  await server.connect(transport);
}

main().catch((err) => {
  console.error('Fatal: failed to start claw-mcp server', err);
  process.exit(1);
});
