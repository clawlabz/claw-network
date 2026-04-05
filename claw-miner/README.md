# claw-miner

ClawNetwork Agent Mining CLI -- earn CLAW by contributing compute resources to the network.

Agent Mining lets AI agents (and humans) participate in ClawNetwork by running a lightweight mining process that sends periodic heartbeats to prove uptime. Rewards are distributed proportionally based on tier, reputation, and activity.

## Quick Start

```bash
# 1. Install
pip install clawminer

# 2. Initialize wallet and config
claw-miner init --name "my-miner" --tier 1

# 3. Start mining
claw-miner start
```

## How It Works

1. **Register**: The miner sends a `MinerRegister` transaction to the chain to register as a Tier 1 (Online) miner.
2. **Heartbeat**: Every ~5 minutes, the miner sends a `MinerHeartbeat` transaction proving it is online and synced.
3. **Earn**: Each block, 35% of the block reward is distributed to active miners proportional to their reputation.
4. **Reputation**: Consistent uptime builds reputation (0-10000 basis points), which increases your share of rewards.

Heartbeat transactions are gas-free -- you only need CLAW for the initial registration.

## CLI Reference

| Command | Description |
|---------|-------------|
| `claw-miner init` | Initialize wallet and config files |
| `claw-miner start` | Start mining (register + heartbeat loop) |
| `claw-miner stop` | Show instructions to stop the miner |
| `claw-miner status` | Show miner registration status and info |
| `claw-miner balance` | Show CLAW balance |
| `claw-miner --version` | Show version |
| `claw-miner --help` | Show help |

### `claw-miner init`

```
Options:
  --dir TEXT   Directory for config and wallet files
  --name TEXT  Miner display name
  --tier INT   Miner tier (only tier 1 supported in Phase 1)
  --rpc TEXT   RPC endpoint URL
```

### `claw-miner start`

```
Options:
  --dir TEXT   Directory for config and wallet files
```

### `claw-miner status` / `claw-miner balance`

```
Options:
  --dir TEXT   Directory for config and wallet files
```

## Miner Tiers

**Phase 1** supports only **Tier 1 (Online)** — a lightweight mining mode that requires minimal resources and sends periodic heartbeats to prove uptime.

Future phases will introduce additional tiers with higher resource requirements and reward structures.

## Configuration

After `claw-miner init`, edit `clawminer.toml` to customize:

```toml
rpc_endpoint = "https://rpc.clawlabz.xyz"
chain_id = "claw-mainnet"
miner_name = "my-miner"
tier = 1
wallet_path = "wallet.json"
```

## Development

### Setup

```bash
git clone https://github.com/clawlabz/claw-network.git
cd claw-network/claw-miner

# Install development dependencies
pip install -e ".[dev]"
```

### Running Tests

```bash
# Run all tests
pytest -v

# Run with coverage report
pytest -v --cov=clawminer
```

## Contributing

1. Fork the repository
2. Create a feature branch (`git checkout -b feature/my-feature`)
3. Write tests first (TDD)
4. Implement the feature
5. Run `pytest -v` and ensure all tests pass
6. Submit a pull request

## License

MIT
