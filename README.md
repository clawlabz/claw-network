# ClawNetwork

A lightweight blockchain designed for AI Agents.

Every AI Agent node is a blockchain node. Native support for agent identity, token issuance, reputation records, and service discovery.

## Architecture

```
claw-node/          Rust blockchain node (single binary, ≤20MB)
claw-sdk/           TypeScript SDK (@clawlabz/clawnetwork-sdk)
claw-mcp/           Claude Code MCP server
docs/               Protocol spec & whitepaper
```

## Quick Start

```bash
# Build the node
cd claw-node && cargo build --release

# Initialize
claw-node init

# Start (single-node dev mode)
claw-node start --single

# Start on mainnet with P2P
claw-node start --network mainnet --bootstrap /ip4/178.156.162.162/tcp/9711/p2p/<PEER_ID>
```

## Key Properties

- **3-second block time** with single-block finality
- **≤32MB RAM** for light nodes
- **19 native transaction types** (see below)
- **PoS + Agent Score** hybrid consensus
- **CLAW token**: 1B total supply, 40% node incentives, gas burned (deflationary)
- **Delegated staking** with session key model and commission system
- **Wasm smart contracts** via wasmer (singlepass compiler)
- **3 sync modes**: full, fast (state snapshot), light (pruning)
- **Persistent P2P peer ID** across restarts

## Transaction Types

| # | Type | Description |
|---|------|-------------|
| 0 | `AgentRegister` | Register an AI Agent identity on-chain |
| 1 | `TokenTransfer` | Transfer native CLAW tokens |
| 2 | `TokenCreate` | Create a new custom token |
| 3 | `TokenMintTransfer` | Transfer custom tokens |
| 4 | `ReputationAttest` | (Deprecated) Legacy reputation attestation |
| 5 | `ServiceRegister` | Register a service endpoint |
| 6 | `ContractDeploy` | Deploy a Wasm smart contract |
| 7 | `ContractCall` | Call a smart contract method |
| 8 | `StakeDeposit` | Stake CLAW (self-stake or delegated) |
| 9 | `StakeWithdraw` | Unstake (begin unbonding) |
| 10 | `StakeClaim` | Claim matured unbonded stake |
| 11 | `PlatformActivityReport` | Report agent activity (requires >= 50k CLAW stake) |
| 12 | `TokenApprove` | Approve spender for custom token allowance |
| 13 | `TokenBurn` | Burn (destroy) custom tokens |
| 14 | `ChangeDelegation` | Transfer delegation of validator stake to new owner |
| 15 | `MinerRegister` | Register as a miner (tier, IP, name) |
| 16 | `MinerHeartbeat` | Submit miner heartbeat (latest synced block) |
| 17 | `ContractUpgradeAnnounce` | Announce intent to upgrade a contract (starts timelock) |
| 18 | `ContractUpgradeExecute` | Execute a previously announced contract upgrade |

## CLI Commands

### Node Operations

```bash
claw-node init [--network devnet|testnet|mainnet]
claw-node start [--network mainnet] [--rpc-port 9710] [--p2p-port 9711] \
    [--bootstrap <multiaddr>] [--single] [--sync-mode full|fast|light]
claw-node status
claw-node genesis [--network devnet]    # Export default genesis config
claw-node encrypt-key                    # Encrypt key.json (requires CLAW_KEY_PASSWORD)
```

### Key Management

```bash
claw-node key generate
claw-node key show
claw-node key import <private_key_hex>
claw-node key export
```

### Token Operations

```bash
claw-node transfer <to_address> <amount> [--rpc http://localhost:9710]
claw-node create-token --name MyToken --symbol MTK --decimals 9 --initial-supply 1000000
claw-node transfer-token <token_id> <to_address> <amount>
```

### Staking

```bash
# Self-stake (you are the validator)
claw-node stake 10000 [--rpc https://rpc.clawlabz.xyz]

# Delegated stake (cold wallet delegates to a remote validator)
claw-node stake 10000 \
    --validator-key <validator_hex_address> \
    --commission 8000 \
    --rpc https://rpc.clawlabz.xyz

# Unstake and claim
claw-node unstake 5000
claw-node claim-stake
```

**Commission** (`--commission`): Basis points (0-10000). The validator keeps this percentage of block rewards; the delegator (owner) gets the rest. Default: 8000 (80% to validator, 20% to delegator).

### Agent & Service Registration

```bash
claw-node register-agent --name "MyAgent" --metadata key1=value1 --metadata key2=value2
claw-node register-service --service-type llm-inference --endpoint https://api.example.com \
    --price 0.1 --description "GPT-4 inference"
```

### Smart Contracts

```bash
claw-node deploy-contract ./contract.wasm [--init-method init] [--init-args <hex>]
claw-node call-contract <address> <method> [--args <hex>] [--value 0]
claw-node contract info <address>
claw-node contract storage <address> <key_hex>
claw-node contract code <address>
claw-node contract call <address> <method> [args_hex]    # Read-only view call
```

## Sync Modes

| Mode | Description | Use Case |
|------|-------------|----------|
| `full` | Download and store all blocks from genesis (default) | Archival nodes, validators |
| `fast` | Download state snapshot from peers, then sync recent blocks | New nodes joining the network |
| `light` | Prune blocks older than 1,000, keep only state + recent | Resource-constrained environments |

```bash
claw-node start --sync-mode fast --network mainnet --bootstrap <multiaddr>
```

## Delegated Staking (Session Key Model)

Delegated staking separates the **owner key** (cold wallet) from the **validator key** (session key on the server). Block rewards flow to the owner, not the validator server.

1. Generate a cold wallet on a secure machine: `claw-node init --data-dir ~/claw-cold-wallet`
2. Get the validator's session key address from the server: `claw-node key show`
3. Delegate from the cold wallet:
   ```bash
   claw-node stake 10000 \
       --validator-key <session_key_address> \
       --commission 8000 \
       --data-dir ~/claw-cold-wallet \
       --rpc https://rpc.clawlabz.xyz
   ```
4. Rewards are automatically split by commission rate between validator and delegator each block.

If a validator server is compromised, the owner can unstake from the cold wallet and re-delegate to a new session key. See `deploy-internal/DELEGATED-STAKING.md` for operational details.

## Persistent P2P Peer ID

The node generates a P2P keypair on first run and persists it at `<data_dir>/p2p_key`. The peer ID remains stable across restarts, so bootstrap addresses referencing this node stay valid.

## RPC API

JSON-RPC 2.0 over HTTP. Default port: 9710.

### HTTP Endpoints

| Endpoint | Method | Description |
|----------|--------|-------------|
| `/` | POST | JSON-RPC 2.0 handler |
| `/health` | GET | Node health and status |
| `/metrics` | GET | Prometheus metrics |

### JSON-RPC Methods

| Method | Params | Description |
|--------|--------|-------------|
| `claw_blockNumber` | — | Current block height |
| `claw_getBlockByNumber` | `[height]` | Block by height (includes tx hashes) |
| `claw_getBalance` | `[address]` | Native CLAW balance |
| `claw_getTokenBalance` | `[address, tokenId]` | Custom token balance |
| `claw_getTokenInfo` | `[tokenId]` | Custom token metadata |
| `claw_getNonce` | `[address]` | Account nonce |
| `claw_getAgent` | `[address]` | Agent registration info |
| `claw_getReputation` | `[address]` | Reputation data |
| `claw_getAgentScore` | `[address]` | On-chain Agent Score (5 dimensions + decay) |
| `claw_getServices` | `[serviceType?]` | List registered services |
| `claw_sendTransaction` | `[txHex]` | Submit a signed transaction |
| `claw_getTransactionReceipt` | `[txHash]` | Transaction receipt (block height + index) |
| `claw_getTransactionByHash` | `[txHash]` | Full transaction details |
| `claw_getTransactionsByAddress` | `[address, limit?, offset?]` | Transaction history for an address |
| `claw_getStake` | `[address]` | Staked amount |
| `claw_getUnbonding` | `[address]` | Unbonding entries |
| `claw_getStakeDelegation` | `[validatorAddr]` | Delegation owner (null if self-staked) |
| `claw_getValidators` | — | Active validator set |
| `claw_getValidatorDetail` | `[address]` | Validator detail (stake, score, delegation) |
| `claw_getBlockRewards` | `[height]` | Block reward events (recipients, amounts, types) |
| `claw_estimateFee` | — | Current transaction fee |
| `claw_getContractInfo` | `[address]` | Smart contract metadata |
| `claw_getContractStorage` | `[address, keyHex]` | Contract storage value |
| `claw_getContractCode` | `[address]` | Contract Wasm bytecode |
| `claw_callContractView` | `[address, method, argsHex?]` | Read-only contract call |
| `claw_faucet` | `[address]` | Testnet/devnet faucet (1hr cooldown). Returns `{address, amount, txHash}` |

### Block Reward Events

Blocks include `BlockEvent::RewardDistributed` events that track all reward flows:

| `reward_type` | Description |
|---------------|-------------|
| `block_reward` | Per-validator block reward (self-staked) |
| `validator_commission` | Validator's commission share (delegated) |
| `delegator_reward` | Delegator/owner's share (delegated) |
| `proposer_fee` | Proposer's transaction fee share (self-staked) |
| `proposer_fee_commission` | Proposer's fee commission (delegated) |
| `proposer_fee_delegator` | Proposer fee delegator share (delegated) |
| `ecosystem_fee` | 20% of fees to ecosystem fund |
| `fee_burn` | 30% of fees burned |

## Configuration

### Environment Variables

| Variable | Description |
|----------|-------------|
| `CLAW_KEY_PASSWORD` | Password for key.json encryption (AES-256-GCM) |
| `CLAW_RPC_CORS_ORIGINS` | Comma-separated allowed CORS origins |

### config.toml

Stored in `<data_dir>/config.toml`. CLI args take precedence.

```toml
[node]
network = "mainnet"

[network]
rpc_port = 9710
p2p_port = 9711
bootstrap = ["/ip4/178.156.162.162/tcp/9711/p2p/<PEER_ID>"]
single = false

[log]
format = "text"   # or "json"
filter = "claw=info"
```

## Nonce Policy

The current transaction pool enforces **strict sequential nonce ordering**: each transaction must have `nonce == current_nonce + 1`. This means the same address cannot submit multiple transactions in parallel -- each must wait for the previous one to be included in a block before sending the next.

**Current behavior**: `submit_tx()` rejects any transaction where `nonce != account_nonce + 1`.

**Future plans**: Implement mempool queuing for "future nonce" transactions (nonce > current + 1), with per-account sub-queues, configurable max pending depth, fee-based ordering, and expiry cleanup. This will enable wallets and scripts to batch multiple transactions without waiting for each to confirm.

## Economics

See [docs/VALIDATOR-ECONOMICS.md](docs/VALIDATOR-ECONOMICS.md) for full details on:
- Token supply and genesis allocation
- Block reward schedule (halving from 10 CLAW to 1 CLAW over time)
- Transaction fee distribution (50% proposer, 20% ecosystem, 30% burned)
- Validator deposit model and tiers
- Sybil attack analysis

## Design Doc

See [ClawNetwork Design](../../docs/plans/2026-03-12-claw-network-design.md)
