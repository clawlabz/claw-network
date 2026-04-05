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
| **Staking** | `client.staking.*` | ✅ Stable | Deposit, withdraw, claim, change delegation |
| **Smart Contracts** | `client.contract.*` | ✅ Stable | Deploy, call smart contracts |
| **Mining** | `client.miner.*` | ✅ Stable | Register miner, submit heartbeats |

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

### `client.staking.*`

Staking and delegation operations.

```typescript
// Deposit stake to become a validator
const txHash = await client.staking.deposit({
  validator: '0xvalidator...',  // validator address
  amount: BigInt(50_000 * 1e9),  // 50,000 CLAW
  commissionBps: 1000  // 10% commission
})

// Withdraw stake (initiates unbonding)
const txHash = await client.staking.withdraw({
  validator: '0xvalidator...',
  amount: BigInt(10_000 * 1e9)
})

// Claim unbonded stake
const txHash = await client.staking.claim()

// Change delegation to a different validator
const txHash = await client.staking.changeDelegation({
  validator: '0xold-validator...',
  newOwner: '0xnew-delegator...',
  commissionBps: 500  // new commission
})
```

### `client.contract.*`

Smart contract operations.

```typescript
// Deploy a new contract
const txHash = await client.contract.deploy({
  code: new Uint8Array([...wasmBytecode]),
  initMethod: 'init',  // constructor method name
  initArgs: new Uint8Array([...constructorArgs])
})

// Call a contract method
const txHash = await client.contract.call({
  contract: '0xcontract-address...',
  method: 'transfer',
  args: new Uint8Array([...methodArgs]),
  value: BigInt(1_000_000)  // optional native CLAW to send
})
```

### `client.miner.*`

Mining operations.

```typescript
// Register as a miner
const txHash = await client.miner.register({
  tier: 1,  // miner tier
  ipAddr: new Uint8Array([192, 168, 1, 1]),  // IPv4 address
  name: 'my-miner'
})

// Submit a heartbeat
const txHash = await client.miner.heartbeat({
  latestBlockHash: '0xblockhash...',
  latestHeight: BigInt(12345)
})
```

### `client.block.*`

Block queries.

```typescript
// Get latest block height
const height = await client.block.getLatest()

// Get block by height
const block = await client.block.getByNumber(12345)
```

### `client.getTransaction(txHash)`

Get full transaction details by hash.

```typescript
const tx = await client.getTransaction('0xtxhash...')
if (tx) {
  console.log(tx.from, tx.to, tx.amount)
}
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

### Phase 1 (Stable)
- ✅ Core transaction signing and serialization
- ✅ Wallet & address derivation
- ✅ Native token transfers
- ✅ Custom token creation & transfer
- ✅ Agent registration & lookup
- ✅ Service registry
- ✅ Block queries & transaction receipts

### Phase 2 (Current — Stable)
- ✅ Staking system (deposit, withdraw, claim, change delegation)
- ✅ Smart contracts (deploy, call)
- ✅ Mining operations (register, heartbeat)
- ✅ Transaction queries (getTransaction, getTransactionReceipt)
- ✅ Full transaction serialization for all 19 tx types

### Phase 3 (Planned)
- 🔶 Enhanced RPC queries (logs, filters, event streaming)
- 🔶 Transaction building utilities
- 🔶 Contract upgrade operations (announce, execute)
- 🔶 Batch transaction utilities

### Phase 4 (Future)
- 📋 Multi-signature transactions
- 📋 Account abstraction
- 📋 WebSocket support for real-time event streaming
- 📋 Advanced contract introspection

## License

MIT — See LICENSE file for details.
