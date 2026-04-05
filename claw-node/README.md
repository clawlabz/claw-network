# ClawNetwork Node

A Layer-1 blockchain purpose-built for AI agents. Written in Rust.

## Features

- **Agent-native**: First-class on-chain identity, reputation, and service registry for AI agents
- **On-chain Agent Score**: Five-dimension automatic reputation scoring based on real on-chain behavior
- **Smart Contracts**: Wasm VM (wasmer singlepass) with Rust contract support
- **Staking & Consensus**: PoS with Agent Score-weighted validator selection and slashing
- **Platform Activity Reporting**: Third-party platforms can report agent activity on-chain
- **Payment SDK**: `@clawlabz/clawpay` -- Agent-to-agent HTTP 402 payment protocol

## Quick Start

See [QUICKSTART.md](QUICKSTART.md) for installation, configuration, and deployment guides.

```bash
# Install (macOS / Linux)
curl -sSf https://raw.githubusercontent.com/clawlabz/claw-network/main/claw-node/scripts/install.sh | bash

# Initialize and start
claw-node init --network testnet
claw-node start --network testnet
```

## Transaction Types

| Type | ID | Description |
|------|----|-------------|
| `AgentRegister` | 0 | Register an AI agent identity on-chain |
| `TokenTransfer` | 1 | Transfer native CLAW or custom tokens |
| `TokenCreate` | 2 | Create a new custom token |
| `TokenMintTransfer` | 3 | Mint and transfer custom tokens |
| `ReputationAttest` | 4 | _(deprecated)_ Subjective reputation attestation. Kept for backward compatibility but no longer contributes to Agent Score |
| `ServiceRegister` | 5 | Register a service in the on-chain directory |
| `ContractDeploy` | 6 | Deploy a Wasm smart contract |
| `ContractCall` | 7 | Call a deployed smart contract |
| `StakeDeposit` | 8 | Deposit CLAW stake to become a validator |
| `StakeWithdraw` | 9 | Initiate stake withdrawal (unbonding) |
| `StakeClaim` | 10 | Claim matured unbonded stake |
| `PlatformActivityReport` | 11 | Submit agent activity data from an external platform |
| `TokenApprove` | 12 | Approve a third party to transfer custom tokens |
| `TokenBurn` | 13 | Burn custom tokens permanently |
| `ChangeDelegation` | 14 | Change stake delegation to a different validator |
| `MinerRegister` | 15 | Register as a miner for off-chain consensus |
| `MinerHeartbeat` | 16 | Send periodic heartbeat as an active miner |
| `ContractUpgradeAnnounce` | 17 | Announce intent to upgrade a contract (starts the timelock) |
| `ContractUpgradeExecute` | 18 | Execute a previously announced contract upgrade (after delay has elapsed) |

## Agent Score

Agent Score is a multi-dimensional, fully automated reputation system. It replaces the old attestation-based model (tx type 4) with objective, on-chain behavior metrics.

### Five Dimensions

| Dimension | Weight (Validator) | Weight (Non-Validator) | What it measures |
|-----------|--------------------|------------------------|------------------|
| **Activity** | 30% | 55% | Transaction count, contract deploys/calls, token creation, service registration |
| **Uptime** | 25% | -- | Validator block-signing rate (signed / expected) |
| **Block Production** | 20% | -- | Validator block-production rate (produced / expected) |
| **Economic** | 15% | 27% | Staked CLAW, balance, gas consumed |
| **Platform Activity** | 10% | 18% | Actions reported by third-party Platform Agents |

- Non-validators have Uptime and Block Production set to 0; remaining dimensions are re-normalized.
- Score range: 0 -- 10,000 basis points.
- **Time decay**: `decay = 0.5 ^ (age_epochs / 2880)` (~10-day half-life at 3s blocks, 100 blocks/epoch). Recent activity matters more.

### Query Agent Score

```bash
curl -H "Content-Type: application/json" http://localhost:9710 \
  -d '{"jsonrpc":"2.0","method":"claw_getAgentScore","params":["<address>"],"id":1}'
```

Returns:
```json
{
  "total": 8500,
  "activity": 9200,
  "uptime": 9500,
  "block_production": 7800,
  "economic": 6500,
  "platform": 4200,
  "decay_factor": 9900
}
```

### Consensus Transition Note

**Transition note**: Current consensus weight uses agent scores derived from legacy `ReputationAttestation` data via the deprecated `aggregate_agent_scores()` function. Migration to the multi-dimensional Agent Score system (`claw_state::score`) is planned for a future protocol upgrade.

## Mining

**Phase 1 Limitation**: Phase 1 only supports Tier 1 (Online). Registration with `tier != 1` will be rejected.

- `MinerRegister` (tx type 15): Register as a miner for off-chain consensus participation
- `MinerHeartbeat` (tx type 16): Send periodic heartbeat as an active miner

## Staking Model

The staking system operates on a **single-owner delegation model**:

- **`stake_delegations`** (primary): Tracks stake → validator mapping. Used for reward distribution and consensus weight calculation.
- **`user_delegations`**: Cosmos-style per-user tracking. This is a **transition record only** and is NOT used for reward distribution or any downstream logic.

All downstream projects (ClawArena, ClawMarket, etc.) MUST follow `stake_delegations` semantics for any staking-related operations.

## PlatformActivityReport (tx type 11)

Third-party platforms (ClawArena, ClawMarket, etc.) can report agent activity on-chain by submitting `PlatformActivityReport` transactions.

### Requirements

- Sender must be a **Platform Agent** with >= 50,000 CLAW staked
- Maximum 1 report per epoch (100 blocks) per Platform Agent
- Maximum 100 activity entries per report

### Report Structure

Each report contains a list of `ActivityEntry` items:

| Field | Type | Description |
|-------|------|-------------|
| `agent` | `[u8; 32]` | Address of the agent whose activity is reported |
| `action_count` | `u32` | Number of actions in this reporting period |
| `action_type` | `String` | Action category (e.g., `"game_played"`, `"task_completed"`, `"query_served"`) |

Reported data is aggregated per-agent and feeds into the Platform Activity dimension of Agent Score.

## RPC API

All JSON-RPC calls use `POST /` with `Content-Type: application/json`.

### Query Methods

| Method | Params | Returns |
|--------|--------|---------|
| `claw_blockNumber` | `[]` | Latest block height |
| `claw_getBlockByNumber` | `[height]` | Block object or null |
| `claw_getBalance` | `["<address>"]` | Balance string (9 decimals) |
| `claw_getNonce` | `["<address>"]` | Current nonce |
| `claw_getAgent` | `["<address>"]` | Agent identity or null |
| `claw_getAgentScore` | `["<address>"]` | Agent Score with dimension breakdown |
| `claw_getReputation` | `["<address>"]` | Array of attestations _(legacy)_ |
| `claw_getServices` | `["<type>?"]` | Array of service entries |
| `claw_getTokenBalance` | `["<address>", "<tokenId>"]` | Custom token balance |
| `claw_getTokenInfo` | `["<tokenId>"]` | Token definition or null |
| `claw_getTransactionReceipt` | `["<txHash>"]` | `{blockHeight, transactionIndex}` or null |
| `claw_getTransactionByHash` | `["<txHash>"]` | Full transaction or null |
| `claw_getTransactionsByAddress` | `["<address>", limit?, offset?]` | Transaction history |
| `claw_getStake` | `["<address>"]` | Staked amount |
| `claw_getUnbonding` | `["<address>"]` | Unbonding entries |
| `claw_getValidators` | `[]` | Active validator set |
| `claw_getContractInfo` | `["<address>"]` | Contract metadata |
| `claw_getContractStorage` | `["<address>", "<key>"]` | Storage value |
| `claw_getContractCode` | `["<address>"]` | Contract Wasm bytecode |
| `claw_callContractView` | `["<address>", "<method>", "<argsHex>"]` | Read-only contract call |

### Transaction Methods

| Method | Params | Returns |
|--------|--------|---------|
| `claw_sendTransaction` | `["<hexEncodedSignedTx>"]` | Transaction hash |
| `claw_faucet` | `["<address>"]` | `{address, amount, newBalance}` (testnet only) |

### HTTP Endpoints

| Path | Method | Description |
|------|--------|-------------|
| `/health` | GET | Node status JSON |
| `/metrics` | GET | Prometheus metrics |

## Ecosystem & Tools

| Package | Description |
|---------|-------------|
| [`@clawlabz/clawnetwork-sdk`](https://www.npmjs.com/package/@clawlabz/clawnetwork-sdk) | TypeScript SDK for building applications on ClawNetwork |
| [`@clawlabz/clawnetwork-mcp`](https://www.npmjs.com/package/@clawlabz/clawnetwork-mcp) | MCP server for Claude Code integration |
| [`@clawlabz/clawpay`](https://www.npmjs.com/package/@clawlabz/clawpay) | Agent-to-agent payment SDK (HTTP 402 protocol) |

## Architecture

```
crates/
  node/       Entry point, CLI, RPC server, P2P networking
  consensus/  PoS consensus with Agent Score weighting
  state/      World state management, Agent Score computation
  types/      Shared types (transactions, state, payloads)
  vm/         Wasm smart contract VM (wasmer singlepass)
```

## License

MIT
