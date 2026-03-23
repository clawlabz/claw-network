# ClawMiner Installation Guide

## For Humans

Copy this into your AI Agent (ChatGPT, Claude, etc.):

```
Help me install and start a ClawNetwork mining node. Follow the instructions at https://github.com/clawlabz/claw-network/blob/main/claw-miner/docs/install.md
```

## For AI Agents

Follow these steps sequentially. Do not skip steps. Do not improvise.

### Step 0: Environment Detection

Run these commands and report the output:

```bash
python3 --version
uname -s -m
pip3 --version
```

**Requirements:**
- Python >= 3.10
- pip (Python package manager)
- Internet connection to ClawNetwork RPC (`https://rpc.clawlabz.xyz`)
- Supported OS: Linux (x86_64, aarch64), macOS (x86_64, arm64), Windows (x86_64)

If Python < 3.10, stop and ask the user to upgrade Python first.

### Step 1: Install clawminer

**From PyPI (recommended):**

```bash
pip install clawminer
```

**From source (development):**

```bash
git clone https://github.com/clawlabz/claw-network.git
cd claw-network/claw-miner
pip install -e .
```

Verify installation:

```bash
claw-miner --version
```

Expected output: `claw-miner, version 0.1.x`

### Step 2: Initialize

```bash
claw-miner init --name "my-agent-miner" --tier 1
```

This creates two files in the current directory:
- `wallet.json` — Ed25519 keypair (KEEP THIS PRIVATE)
- `clawminer.toml` — configuration file

**Tier options:**

| Tier | Name | Description |
|------|------|-------------|
| 0 | Light | Minimal resources, lowest rewards |
| 1 | Standard | Default tier, moderate rewards |
| 2 | Full | Full node sync, higher rewards |
| 3 | Archive | Full history, highest rewards |

### Step 3: Fund your wallet

Get your address:

```bash
claw-miner status
```

Transfer CLAW to the displayed address, or use the testnet faucet.

### Step 4: Start mining

```bash
claw-miner start
```

The miner will:
1. Register on-chain (if not already registered)
2. Send heartbeats every ~50 minutes
3. Earn CLAW rewards proportional to uptime and tier

### Step 5: Verify

```bash
claw-miner balance
```

You should see your CLAW balance increasing over time.

## Configuration

Config file: `clawminer.toml`

| Key | Default | Description |
|-----|---------|-------------|
| `rpc_endpoint` | `https://rpc.clawlabz.xyz` | Node RPC URL |
| `chain_id` | `claw-mainnet` | Network identifier |
| `miner_name` | `claw-miner` | Display name |
| `tier` | `1` | 0=Light, 1=Standard, 2=Full, 3=Archive |
| `wallet_path` | `wallet.json` | Path to wallet file |

## Boundaries

**DO NOT:**
- Run with `sudo` or as root
- Modify any system files outside the working directory
- Share or commit `wallet.json` to version control
- Modify `wallet.json` manually

**SAFE operations:**
- Read/write files in the current working directory only
- Make HTTP requests to the configured RPC endpoint
- Create `wallet.json` and `clawminer.toml` in the current directory

## Troubleshooting

### `command not found: claw-miner`

Ensure pip's script directory is in your PATH:

```bash
# Check where pip installs scripts
python3 -m site --user-base
# Add to PATH (append /bin on macOS/Linux)
export PATH="$PATH:$(python3 -m site --user-base)/bin"
```

### Connection refused

Check that `rpc_endpoint` in `clawminer.toml` points to a running ClawNetwork node.

```bash
curl -s https://rpc.clawlabz.xyz/block_number
```

If this fails, the RPC endpoint may be down.

### Registration failed

Ensure your wallet has enough CLAW for the registration transaction gas fee.

### Heartbeat failed

Network hiccup — the miner will retry automatically on the next cycle (~50 minutes).

### `ModuleNotFoundError`

Reinstall dependencies:

```bash
pip install --force-reinstall clawminer
```

### Python version too old

```bash
python3 --version
```

If below 3.10, upgrade Python. On Ubuntu/Debian: `apt install python3.10`. On macOS: `brew install python@3.12`.
