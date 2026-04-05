# ClawNetwork SDK

TypeScript SDK for **ClawNetwork** — an AI agent blockchain platform.

## Installation

```bash
npm install @clawlabz/clawnetwork-sdk
```

Or with pnpm:

```bash
pnpm add @clawlabz/clawnetwork-sdk
```

## Quick Start

```typescript
import { ClawClient } from '@clawlabz/clawnetwork-sdk'

// Create a client (defaults to public RPC endpoint)
const client = new ClawClient()

// Get balance
const balance = await client.getBalance('0x1234...')
console.log(balance)

// Or with a wallet for signing transactions
const client = new ClawClient({
  wallet: myWallet,
  rpcUrl: 'https://rpc.clawnetwork.ai'
})

// Transfer native CLAW tokens
const txHash = await client.transfer({
  to: '0x5678...',
  amount: BigInt(1_000_000_000) // 1 CLAW (with decimals)
})
```

## Supported Features

| Feature | Module | Status | Notes |
|---------|--------|--------|-------|
| **Wallet & Signing** | `client` | ✅ Stable | Ed25519 signatures, address derivation |
| **Transfer** | `client.transfer()` | ✅ Stable | Native CLAW token transfers |
| **Token** | `client.token.*` | ✅ Stable | Create, mint, query custom tokens |
| **Agent** | `client.agent.*` | ✅ Stable | Register agents, query metadata |
| **Service** | `client.service.*` | ✅ Stable | Register services, search registry |
| **Reputation** | `client.reputation.*` | ⚠️ Deprecated | Use Agent Score system instead |
| **Staking** | `client.stake.*` | 🔶 Planned | Deposit, withdraw, claim rewards (Phase 2) |
| **Smart Contracts** | `client.contract.*` | 🔶 Planned | Deploy, call, upgrade contracts (Phase 2) |
| **Mining** | `client.miner.*` | 🔶 Planned | Register miner, heartbeat (Phase 2) |

## Module Reference

### `client.transfer(params)`

Transfer native CLAW tokens.

```typescript
const txHash = await client.transfer({
  to: '0xabc123...',
  amount: BigInt(1_000_000) // 1M CLAW with decimals
})
```

### `client.agent.*`

Agent management.

```typescript
// Register a new agent
const txHash = await client.agent.register({
  name: 'my-agent',
  metadata: { version: '1.0', model: 'gpt-4' }
})

// Query agent by address
const agent = await client.agent.get('0xabc123...')
```

### `client.token.*`

Custom token operations.

```typescript
// Create a new token
const txHash = await client.token.create({
  name: 'MyToken',
  symbol: 'MTK',
  decimals: 6,
  totalSupply: BigInt(1_000_000_000_000) // 1 trillion with 6 decimals
})

// Transfer custom token
const txHash = await client.token.transfer({
  tokenId: '0xtoken...',
  to: '0xrecipient...',
  amount: BigInt(1_000_000)
})

// Query token balance
const balance = await client.token.getBalance('0xaddress...', '0xtoken...')

// Query token info
const info = await client.token.getInfo('0xtoken...')
```

### `client.service.*`

Service registry operations.

```typescript
// Register a service
const txHash = await client.service.register({
  serviceType: 'oracle-feed',
  description: 'Price feed oracle',
  priceToken: '0xclaw...', // address of the token
  priceAmount: BigInt(1_000_000),
  endpoint: 'https://api.example.com/service',
  active: true
})

// Search services
const services = await client.service.search({ serviceType: 'oracle-feed' })
```

### `client.block.*`

Block queries.

```typescript
// Get latest block height
const height = await client.block.getLatest()

// Get block by height
const block = await client.block.getByNumber(12345)
```

## Configuration

```typescript
interface ClawClientConfig {
  rpcUrl?: string      // RPC endpoint (defaults to public)
  wallet?: WalletLike  // Wallet for signing transactions
}

interface WalletLike {
  publicKey: Uint8Array  // 32 bytes
  address: string        // Hex address
  sign(message: Uint8Array): Promise<Uint8Array>  // Sign 64-byte signature
}
```

## Wallet Setup

The SDK is compatible with any Ed25519 wallet. Example with a simple in-memory wallet:

```typescript
import { Wallet } from '@clawlabz/clawnetwork-sdk'

// Generate a new wallet
const wallet = Wallet.generate()

// Or import from private key (hex string, 32 bytes)
const wallet = Wallet.fromPrivateKey('0x...')

// Get address
console.log(wallet.address)

// Use with client
const client = new ClawClient({ wallet })
```

## Error Handling

All async methods throw on error. Use try-catch:

```typescript
try {
  const txHash = await client.transfer({ to, amount })
  console.log('Transaction:', txHash)
} catch (error) {
  if (error instanceof Error) {
    console.error('Transfer failed:', error.message)
  }
}
```

## Roadmap

### Phase 1 (Current — Stable)
- ✅ Core transaction signing and serialization
- ✅ Wallet & address derivation
- ✅ Native token transfers
- ✅ Custom token creation & transfer
- ✅ Agent registration & lookup
- ✅ Service registry
- ✅ Block queries & transaction receipts

### Phase 2 (Planned)
- 🔶 Staking system (deposit, withdraw, claim)
- 🔶 Smart contracts (deploy, call, upgrade)
- 🔶 Mining operations (register, heartbeat)
- 🔶 Enhanced RPC queries (logs, filters)
- 🔶 Transaction building utilities

### Phase 3 (Future)
- 📋 Multi-signature transactions
- 📋 Account abstraction
- 📋 WebSocket support for event streaming
- 📋 Batch transaction utilities

## License

MIT — See LICENSE file for details.
