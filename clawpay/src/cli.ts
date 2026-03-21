#!/usr/bin/env node

/**
 * ClawPay CLI — wallet management, balance queries, transfers, service discovery.
 *
 * Commands:
 *   clawpay wallet create           — Generate a new wallet
 *   clawpay wallet import <key>     — Import from private key
 *   clawpay balance [address]       — Query CLAW balance
 *   clawpay send <to> <amount>      — Transfer CLAW
 *   clawpay services [--type <t>]   — Discover registered services
 */

import { Wallet, bytesToHex } from './core/wallet.js';
import { RpcClient } from './core/rpc.js';
import { buildTransferTx, formatAmount } from './core/transaction.js';
import { RPC_MAINNET, RPC_TESTNET, CLAW_DECIMALS } from './core/types.js';

// ---------------------------------------------------------------------------
// Argument parsing (zero-dependency)
// ---------------------------------------------------------------------------

interface ParsedArgs {
  readonly command: string;
  readonly subcommand: string;
  readonly positional: string[];
  readonly flags: Record<string, string>;
}

function parseArgs(argv: string[]): ParsedArgs {
  const args = argv.slice(2); // skip node + script
  const positional: string[] = [];
  const flags: Record<string, string> = {};

  for (let i = 0; i < args.length; i++) {
    const arg = args[i];
    if (arg.startsWith('--')) {
      const key = arg.slice(2);
      const next = args[i + 1];
      if (next && !next.startsWith('--')) {
        flags[key] = next;
        i++;
      } else {
        flags[key] = 'true';
      }
    } else {
      positional.push(arg);
    }
  }

  return {
    command: positional[0] ?? '',
    subcommand: positional[1] ?? '',
    positional: positional.slice(2),
    flags,
  };
}

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

async function walletCreate(): Promise<void> {
  const wallet = await Wallet.generate();
  const data = wallet.toJSON();
  console.log('Wallet created successfully!\n');
  console.log(`  Address:     ${data.address}`);
  console.log(`  Public Key:  ${data.publicKey}`);
  console.log(`  Private Key: ${data.privateKey}`);
  console.log('\nSave your private key securely. It cannot be recovered.');
}

async function walletImport(privateKeyHex: string): Promise<void> {
  if (!privateKeyHex) {
    console.error('Usage: clawpay wallet import <private_key_hex>');
    process.exit(1);
  }
  const wallet = await Wallet.fromPrivateKey(privateKeyHex);
  const data = wallet.toJSON();
  console.log('Wallet imported successfully!\n');
  console.log(`  Address:     ${data.address}`);
  console.log(`  Public Key:  ${data.publicKey}`);
}

async function queryBalance(
  address: string | undefined,
  rpcUrl: string,
): Promise<void> {
  if (!address) {
    console.error('Usage: clawpay balance <address>');
    console.error('  or set AGENT_KEY env var to use your wallet address');
    process.exit(1);
  }

  const rpc = new RpcClient({ url: rpcUrl });
  const balance = await rpc.getBalance(address);
  const formatted = formatAmount(balance, CLAW_DECIMALS);
  console.log(`Balance: ${formatted} CLAW`);
  console.log(`  (${balance.toString()} base units)`);
}

async function sendTransfer(
  to: string,
  amount: string,
  privateKey: string,
  rpcUrl: string,
  token?: string,
): Promise<void> {
  if (!to || !amount) {
    console.error('Usage: clawpay send <to_address> <amount> [--token CLAW]');
    process.exit(1);
  }
  if (!privateKey) {
    console.error('Error: AGENT_KEY environment variable is required for sending.');
    process.exit(1);
  }

  const wallet = await Wallet.fromPrivateKey(privateKey);
  const rpc = new RpcClient({ url: rpcUrl });

  console.log(`Sending ${amount} ${token ?? 'CLAW'} to ${to}...`);

  // Get current nonce
  const nonce = await rpc.getNonce(wallet.address);

  // Build and sign transaction
  const { hash: txHex } = await buildTransferTx(wallet, nonce + 1n, { to, amount });

  // Submit
  const txHash = await rpc.sendTransaction(txHex);
  console.log(`Transaction submitted: ${txHash}`);

  // Wait for confirmation
  console.log('Waiting for confirmation...');
  const receipt = await rpc.waitForConfirmation(txHash, 15_000, 1_000);
  console.log(`Confirmed in block ${receipt.blockHeight} (index ${receipt.transactionIndex})`);
}

async function discoverServices(
  rpcUrl: string,
  serviceType?: string,
): Promise<void> {
  const rpc = new RpcClient({ url: rpcUrl });
  const services = await rpc.getServices(serviceType);

  if (services.length === 0) {
    console.log(serviceType
      ? `No services found of type "${serviceType}".`
      : 'No services registered.');
    return;
  }

  console.log(`Found ${services.length} service(s):\n`);
  for (const svc of services) {
    console.log(`  Type:        ${svc.service_type}`);
    console.log(`  Provider:    ${svc.provider}`);
    console.log(`  Description: ${svc.description}`);
    console.log(`  Price:       ${svc.price_amount} (token: ${svc.price_token})`);
    console.log(`  Endpoint:    ${svc.endpoint}`);
    console.log(`  Active:      ${svc.active}`);
    console.log('');
  }
}

// ---------------------------------------------------------------------------
// Main
// ---------------------------------------------------------------------------

async function main(): Promise<void> {
  const parsed = parseArgs(process.argv);
  const rpcUrl = parsed.flags['rpc'] ?? (parsed.flags['testnet'] ? RPC_TESTNET : RPC_MAINNET);
  const privateKey = parsed.flags['key'] ?? process.env.AGENT_KEY ?? '';

  try {
    switch (parsed.command) {
      case 'wallet': {
        switch (parsed.subcommand) {
          case 'create':
            await walletCreate();
            break;
          case 'import':
            await walletImport(parsed.positional[0]);
            break;
          default:
            console.error('Usage: clawpay wallet <create|import>');
            process.exit(1);
        }
        break;
      }

      case 'balance': {
        let address = parsed.subcommand || undefined;
        if (!address && privateKey) {
          const wallet = await Wallet.fromPrivateKey(privateKey);
          address = wallet.address;
        }
        await queryBalance(address, rpcUrl);
        break;
      }

      case 'send': {
        const to = parsed.subcommand;
        const amount = parsed.positional[0];
        await sendTransfer(to, amount, privateKey, rpcUrl, parsed.flags['token']);
        break;
      }

      case 'services': {
        await discoverServices(rpcUrl, parsed.flags['type'] ?? (parsed.subcommand || undefined));
        break;
      }

      case 'help':
      case '--help':
      case '-h':
      case '': {
        printUsage();
        break;
      }

      default:
        console.error(`Unknown command: ${parsed.command}`);
        printUsage();
        process.exit(1);
    }
  } catch (err) {
    console.error(`Error: ${err instanceof Error ? err.message : String(err)}`);
    process.exit(1);
  }
}

function printUsage(): void {
  console.log(`
ClawPay CLI — ClawNetwork payment tools for AI Agents

Usage:
  clawpay wallet create                 Generate a new Ed25519 wallet
  clawpay wallet import <private_key>   Import wallet from private key hex

  clawpay balance [address]             Query CLAW balance
  clawpay send <to> <amount>            Send CLAW tokens

  clawpay services [--type <type>]      Discover registered services

Options:
  --rpc <url>       RPC endpoint (default: mainnet)
  --testnet         Use testnet RPC
  --key <hex>       Private key (or set AGENT_KEY env var)
  --token <symbol>  Token symbol (default: CLAW)
  --type <type>     Filter services by type

Examples:
  clawpay wallet create
  clawpay balance abc123...
  clawpay send abc123... 10 --key def456...
  clawpay services --type translation
`.trim());
}

main();
