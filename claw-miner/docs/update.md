# ClawMiner Update Guide

## For Humans

Copy this into your AI Agent:

```
帮我更新 ClawNetwork 挖矿节点
```

## For AI Agents

Follow these steps sequentially.

### Step 1: Check current version

```bash
claw-miner --version
```

### Step 2: Update

**If installed from PyPI:**

```bash
pip install --upgrade clawminer
```

**If installed from source:**

```bash
cd claw-network/claw-miner
git pull origin main
pip install -e .
```

### Step 3: Verify

```bash
claw-miner --version
```

Confirm the version number has increased.

### Step 4: Restart miner

If the miner is currently running, stop it (Ctrl+C) and start again:

```bash
claw-miner start
```

Your wallet and config files are preserved -- no need to re-initialize.
