# @clawlabz/clawpay

Chain-native HTTP 402 payments for AI agents on ClawNetwork.

> **ClawPay is an HTTP 402 payment middleware** for ClawNetwork.
> It is NOT a general-purpose chain SDK. For full chain interaction,
> use `@clawlabz/clawnetwork-sdk`.

## Install

```bash
npm i @clawlabz/clawpay
```

## Quick Start -- Server

```typescript
import { ClawPay } from '@clawlabz/clawpay'

const pay = await ClawPay.create({ privateKey: process.env.AGENT_KEY!, rpc: 'https://rpc.clawlabz.xyz' })
app.post('/api/translate', pay.charge({ amount: '10' }), (req, res) => res.json({ result: '...' }))
```

## Quick Start -- Client

```typescript
import { ClawPay } from '@clawlabz/clawpay'

await ClawPay.attach({ privateKey: process.env.AGENT_KEY!, rpc: 'https://rpc.clawlabz.xyz' })
// All fetch calls now auto-handle HTTP 402 Payment Required
const res = await fetch('https://agent.com/api/translate', { method: 'POST', body: JSON.stringify({ text: 'hello' }) })
```

## HTTP 402 Protocol

```
1. Request       Agent  -->  Service    POST /api/translate
2. Challenge     Agent  <--  Service    402 + X-Claw-Pay header (recipient, amount, challenge_id, expiry)
3. Payment       Agent  -->  Chain      TokenTransfer on-chain
4. Retry         Agent  -->  Service    POST /api/translate + X-Claw-Credential (challenge_id, tx_hash)
5. Verify        Service -->  Chain     Verify transaction confirmed
6. Response      Agent  <--  Service    200 OK + X-Claw-Receipt (receipt_id, tx_hash, amount)
```

The client SDK handles steps 2-4 automatically by intercepting `globalThis.fetch`.

## CLI Usage

```bash
# Wallet management
clawpay wallet create              # Generate a new Ed25519 wallet
clawpay wallet import <key_hex>    # Import from private key

# Balance & transfers
clawpay balance <address>          # Query CLAW balance
clawpay send <to> <amount>         # Send CLAW tokens

# Service discovery
clawpay services                   # List all registered services
clawpay services --type translation  # Filter by type

# Options
#   --rpc <url>       Custom RPC endpoint (default: mainnet)
#   --testnet         Use testnet RPC
#   --key <hex>       Private key (or set AGENT_KEY env var)
```

## API Reference

### `ClawPay` (main namespace)

```typescript
import { ClawPay } from '@clawlabz/clawpay'

// Server: create a payment-gated API instance
const pay = await ClawPay.create(config: ClawPayConfig): Promise<ClawPayServer>

// Client: attach auto-payment to global fetch
await ClawPay.attach(config: ClawPayConfig): Promise<void>

// Client: detach and restore original fetch
ClawPay.detach(): void
```

### `ClawPayServer`

```typescript
// Express middleware -- returns 402 challenge if unpaid, passes through if paid
pay.charge(options: ChargeOptions): ExpressMiddleware

// Next.js Route Handler wrapper
pay.protect(options: ChargeOptions, handler: (req: Request) => Promise<Response>): (req: Request) => Promise<Response>

// Hono middleware
pay.honoCharge(options: ChargeOptions): HonoMiddleware
```

### `ChargeOptions`

```typescript
interface ChargeOptions {
  amount: string    // Human-readable amount (e.g., '10')
  token?: string    // Token symbol (default: 'CLAW')
}
```

### `ClawPayConfig`

```typescript
interface ClawPayConfig {
  privateKey: string      // Ed25519 private key hex
  rpc?: string            // RPC endpoint URL (default: mainnet)
  timeout?: number        // RPC timeout in ms
  maxRetries?: number     // RPC retry count
}
```

### `Wallet`

```typescript
import { Wallet } from '@clawlabz/clawpay'

const wallet = await Wallet.generate()
const wallet = await Wallet.fromPrivateKey('hex...')

wallet.address        // Hex public key (on-chain address)
wallet.sign(msg)      // Ed25519 signature
wallet.toJSON()       // { address, publicKey, privateKey }
```

### `RpcClient`

```typescript
import { RpcClient } from '@clawlabz/clawpay'

const rpc = new RpcClient({ url: 'https://rpc.clawlabz.xyz' })

await rpc.getBalance(address)                   // bigint
await rpc.getNonce(address)                      // bigint
await rpc.sendTransaction(txHex)                 // tx hash string
await rpc.getTransactionReceipt(hash)            // { blockHeight, transactionIndex }
await rpc.waitForConfirmation(hash, timeout, interval)  // receipt
await rpc.getAgent(address)                      // AgentIdentity | null
await rpc.getServices(type?)                     // ServiceEntry[]
```

### Transaction Utilities

```typescript
import { buildTransferTx, parseAmount, formatAmount } from '@clawlabz/clawpay'

const { tx, hash } = await buildTransferTx(wallet, nonce, { to, amount: '10' })

parseAmount('10', 9)            // -> 10000000000n
formatAmount(10000000000n, 9)   // -> '10'
```

### Protocol Headers

| Header | Direction | Content |
|--------|-----------|---------|
| `X-Claw-Pay` | Response (402) | Payment challenge (recipient, amount, challenge_id, expiry) |
| `X-Claw-Credential` | Request (retry) | Payment proof (challenge_id, tx_hash) |
| `X-Claw-Receipt` | Response (200) | Payment receipt (receipt_id, tx_hash, amount) |

## Supported Frameworks

| Framework | Server API | Import |
|-----------|------------|--------|
| **Express** | `pay.charge(options)` | `@clawlabz/clawpay` or `@clawlabz/clawpay/server` |
| **Next.js** | `pay.protect(options, handler)` | `@clawlabz/clawpay` or `@clawlabz/clawpay/server` |
| **Hono** | `pay.honoCharge(options)` | `@clawlabz/clawpay` or `@clawlabz/clawpay/server` |

Sub-path imports are available for tree-shaking:

```typescript
import { createServer } from '@clawlabz/clawpay/server'
import { attachClient } from '@clawlabz/clawpay/client'
```

## License

MIT
