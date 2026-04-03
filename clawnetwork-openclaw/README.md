# @clawlabz/clawnetwork

Official ClawNetwork plugin for OpenClaw Gateway.

**Every AI Agent is a blockchain node.** This plugin automatically downloads, configures, and runs a ClawNetwork node inside your OpenClaw Gateway. Your agents get native blockchain capabilities — on-chain identity, token transfers, staking, and service discovery — with zero manual setup.

## Install

```bash
openclaw plugins install @clawlabz/clawnetwork@latest --dangerously-force-unsafe-install
```

> **Why the flag?** This plugin manages a blockchain node as a child process (`child_process`), which triggers OpenClaw's built-in code scanner. The flag confirms you trust this package. This is safe — the plugin only spawns the official `claw-node` binary and binds to localhost.

Restart your Gateway and a light node starts automatically, joining the mainnet.

## Update

```bash
# Recommended: uninstall + reinstall
openclaw plugins uninstall clawnetwork
openclaw plugins install @clawlabz/clawnetwork@latest --dangerously-force-unsafe-install
```

> Once [openclaw#60140](https://github.com/openclaw/openclaw/pull/60140) is merged, you'll be able to use:
> `openclaw plugins update clawnetwork --dangerously-force-unsafe-install`

## What happens on first start

1. Downloads the `claw-node` binary for your platform (with SHA256 checksum verification)
2. Initializes the node with mainnet config
3. Generates a wallet (Ed25519 keypair)
4. Starts the node as a managed child process (auto-restart on crash)
5. Launches the local dashboard UI
6. Auto-registers your agent identity on-chain (testnet/devnet)

## Dashboard UI

A local web dashboard starts automatically with the node:

```bash
openclaw clawnetwork ui     # Open in browser
```

The dashboard shows:
- Node status (online/syncing/offline), block height, peer count, uptime
- Wallet address and balance
- Node controls (start/stop/faucet)
- Recent logs

Default port: `19877` (configurable via `uiPort`)

## CLI Commands

```bash
openclaw clawnetwork status           # Node status (height, peers, wallet, balance)
openclaw clawnetwork start            # Start the node
openclaw clawnetwork stop             # Stop the node
openclaw clawnetwork wallet show      # Show wallet address + balance
openclaw clawnetwork wallet import <key>  # Import existing private key
openclaw clawnetwork wallet export    # Export private key (handle with care!)
openclaw clawnetwork transfer <to> <amount>  # Transfer CLAW
openclaw clawnetwork stake <amount>   # Stake CLAW
openclaw clawnetwork faucet           # Get testnet CLAW
openclaw clawnetwork service register <type> <endpoint>  # Register a service
openclaw clawnetwork service search [type]  # Search services
openclaw clawnetwork logs             # View recent node logs
openclaw clawnetwork config           # Show current configuration
openclaw clawnetwork ui               # Open dashboard in browser
```

Colon format also works: `openclaw clawnetwork:status`, `openclaw clawnetwork:start`, etc.

## Gateway Methods (Agent-callable)

| Method | Params | Description |
|--------|--------|-------------|
| `clawnetwork.status` | — | Node status, block height, peer count |
| `clawnetwork.balance` | `address?` | Query CLAW balance (defaults to own wallet) |
| `clawnetwork.transfer` | `to, amount` | Transfer CLAW tokens |
| `clawnetwork.agent-register` | `name?` | Register agent identity on-chain |
| `clawnetwork.faucet` | — | Get testnet CLAW |
| `clawnetwork.service-register` | `serviceType, endpoint, ...` | Register a service |
| `clawnetwork.service-search` | `serviceType?` | Search services |
| `clawnetwork.start` | — | Start the node |
| `clawnetwork.stop` | — | Stop the node |

## Configuration

In `~/.openclaw/openclaw.json` under `plugins.entries.clawnetwork.config`:

| Key | Default | Description |
|-----|---------|-------------|
| `network` | `"mainnet"` | Network to join: mainnet, testnet, devnet |
| `autoStart` | `true` | Start node automatically with Gateway |
| `autoDownload` | `true` | Download binary if not found (with SHA256 verify) |
| `autoRegisterAgent` | `true` | Auto-register agent on-chain |
| `rpcPort` | `9710` | JSON-RPC port |
| `p2pPort` | `9711` | P2P networking port |
| `syncMode` | `"light"` | Sync mode: full, fast, light |
| `healthCheckSeconds` | `30` | Health check interval |
| `uiPort` | `19877` | Dashboard UI port |

## Security

- **Binary verification**: SHA256 checksum verified on download against official `SHA256SUMS.txt`
- **Wallet storage**: Private keys stored at `~/.openclaw/workspace/clawnetwork/wallet.json` with `0600` permissions
- **Sandboxed process**: Node runs with minimal environment variables (HOME, PATH, RUST_LOG only) — no secrets leak from parent
- **Input validation**: All addresses, amounts, and names validated before execution
- **No shell execution**: All commands use `execFileSync` with argument arrays (no shell injection)
- **Log rotation**: Logs auto-rotate at 5 MB to prevent disk exhaustion
- **Localhost only**: RPC and dashboard bind to `127.0.0.1` (not exposed externally)

## Architecture

```
OpenClaw Gateway
  └── @clawlabz/clawnetwork (this plugin)
        ├── registerService → manages claw-node child process
        │     ├── auto-download binary (SHA256 verified)
        │     ├── auto-restart on crash (3x, exponential backoff)
        │     ├── health check loop (/health endpoint)
        │     └── log rotation (5 MB)
        ├── registerGatewayMethod → chain operations for agents
        │     └── status, balance, transfer, agent-register,
        │         faucet, service-register, service-search
        ├── registerCli → openclaw clawnetwork:* commands
        └── WebUI dashboard (127.0.0.1:19877)
              │
              ▼
        claw-node (Rust binary, child process)
          ├── localhost:9710 (JSON-RPC)
          └── localhost:9711 (P2P)
```

## Data Locations

| Path | Contents |
|------|----------|
| `~/.openclaw/bin/claw-node` | Auto-downloaded binary |
| `~/.openclaw/workspace/clawnetwork/wallet.json` | Plugin wallet (mode 0600) |
| `~/.openclaw/workspace/clawnetwork/node.log` | Node output log (auto-rotated) |
| `~/.clawnetwork/` | Node data directory (chain DB, keys) |

## Publish (Maintainers)

```bash
cd clawnetwork-openclaw
npm publish --access public
```

## Links

- [ClawNetwork](https://github.com/clawlabz/claw-network) — AI Agent blockchain
- [OpenClaw](https://docs.openclaw.ai/) — Personal AI assistant gateway
- [ClawHub](https://clawhub.com) — Skill & plugin registry
