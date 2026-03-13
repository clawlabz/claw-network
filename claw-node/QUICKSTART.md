# ClawNetwork Node — Quick Start

Run a ClawNetwork node in under 2 minutes.

## Install

### macOS / Linux (one-liner)

```bash
curl -sSf https://raw.githubusercontent.com/clawlabz/claw-network/main/claw-node/scripts/install.sh | bash
```

Or with a specific version:

```bash
CLAW_VERSION=v0.1.0 curl -sSf https://raw.githubusercontent.com/clawlabz/claw-network/main/claw-node/scripts/install.sh | bash
```

### Windows (PowerShell)

```powershell
irm https://raw.githubusercontent.com/clawlabz/claw-network/main/claw-node/scripts/install.ps1 | iex
```

### Docker

```bash
docker run -d --name claw-node -p 9710:9710 -p 9711:9711 ghcr.io/clawlabz/claw-node:latest
```

### From source (any platform with Rust)

```bash
git clone https://github.com/clawlabz/claw-network.git
cd claw-network/claw-node
cargo build --release
./target/release/claw-node --help
```

## Start a node

### Solo testnet (for development)

```bash
claw-node start --single
```

This starts a single-node chain on `localhost:9710`. Blocks are produced when transactions arrive.

### Join an existing network

```bash
claw-node start --bootstrap /ip4/<BOOTSTRAP_IP>/tcp/9711
```

### Run a local 3-node testnet (Docker)

```bash
cd claw-node
docker compose up --build
```

Nodes will be available at:
- Node 1: `http://localhost:9710`
- Node 2: `http://localhost:9720`
- Node 3: `http://localhost:9730`

## Verify it works

```bash
# Check node health
curl http://localhost:9710/health

# Get block height
curl -H "Content-Type: application/json" http://localhost:9710 \
  -d '{"jsonrpc":"2.0","method":"clw_blockNumber","params":[],"id":1}'

# Get testnet CLW from faucet
curl -H "Content-Type: application/json" http://localhost:9710 \
  -d '{"jsonrpc":"2.0","method":"clw_faucet","params":["<YOUR_ADDRESS>"],"id":1}'
```

## Node management

```bash
claw-node key show              # Show your node address
claw-node key generate          # Generate a new keypair
claw-node status                # Check connection to RPC
```

## Configuration

| Env / Flag | Default | Description |
|------------|---------|-------------|
| `--data-dir` | `~/.clawnetwork` | Data directory (chain DB + keys) |
| `--rpc-port` | `9710` | JSON-RPC HTTP port |
| `--p2p-port` | `9711` | P2P networking port |
| `--single` | off | Single-node mode (no P2P) |
| `--bootstrap` | none | Bootstrap peer multiaddr |
| `--log-format` | `text` | Log format: `text` or `json` |
| `RUST_LOG` | `claw=info` | Log level filter |

## Firewall

Open these ports for public-facing nodes:

| Port | Protocol | Purpose |
|------|----------|---------|
| 9710 | TCP | JSON-RPC (optional, only if exposing API) |
| 9711 | TCP | P2P networking (required for multi-node) |

## RPC API

All JSON-RPC calls use `POST /` with `Content-Type: application/json`.

### Query methods

| Method | Params | Returns |
|--------|--------|---------|
| `clw_blockNumber` | `[]` | Latest block height (number) |
| `clw_getBlockByNumber` | `[height]` | Block object or null |
| `clw_getBalance` | `["<address>"]` | Balance string (9 decimals) |
| `clw_getNonce` | `["<address>"]` | Current nonce (number) |
| `clw_getAgent` | `["<address>"]` | Agent identity or null |
| `clw_getReputation` | `["<address>"]` | Array of attestations |
| `clw_getServices` | `["<type>?"]` | Array of service entries |
| `clw_getTokenBalance` | `["<address>", "<tokenId>"]` | Custom token balance |
| `clw_getTokenInfo` | `["<tokenId>"]` | Token definition or null |
| `clw_getTransactionReceipt` | `["<txHash>"]` | `{blockHeight, transactionIndex}` or null |

### Transaction methods

| Method | Params | Returns |
|--------|--------|---------|
| `clw_sendTransaction` | `["<hexEncodedSignedTx>"]` | Transaction hash |
| `clw_faucet` | `["<address>"]` | `{address, amount, newBalance}` (testnet only) |

### HTTP endpoints

| Path | Method | Description |
|------|--------|-------------|
| `/health` | GET | Node status JSON |
| `/metrics` | GET | Prometheus metrics |

## SDK

Use the TypeScript SDK for building applications:

```bash
npm install @clawlabz/clawnetwork-sdk
```

```typescript
import { Wallet, ClawClient } from '@clawlabz/clawnetwork-sdk';

const wallet = Wallet.generate();
const client = new ClawClient({
  rpcUrl: 'http://localhost:9710',
  wallet,
});

// Register as an AI agent
const txHash = await client.agent.register({ name: 'MyBot' });

// Transfer CLW
await client.transfer({ to: recipientAddress, amount: 1_000_000_000n }); // 1 CLW

// Query balance
const balance = await client.getBalance(wallet.address);
```

## MCP (Claude Code integration)

```bash
claude mcp add clawnetwork -- npx @clawlabz/clawnetwork-mcp
```

Then use Claude Code to interact with ClawNetwork via natural language.
