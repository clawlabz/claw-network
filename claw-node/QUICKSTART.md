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

## Run as a Service (Production)

The node should run as a background service so it persists after SSH disconnect and auto-restarts on failure.

### Linux (systemd) — Recommended

```bash
# Create service file
sudo tee /etc/systemd/system/claw-node.service > /dev/null << 'EOF'
[Unit]
Description=ClawNetwork Node
Documentation=https://github.com/clawlabz/claw-network
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
User=root
ExecStart=/usr/local/bin/claw-node start --network testnet
Restart=on-failure
RestartSec=5
LimitNOFILE=65535

# Logging (stdout captured by journald)
StandardOutput=journal
StandardError=journal
SyslogIdentifier=claw-node

[Install]
WantedBy=multi-user.target
EOF

# Enable and start
sudo systemctl daemon-reload
sudo systemctl enable --now claw-node

# View logs
journalctl -u claw-node -f

# Common commands
sudo systemctl status claw-node    # Check status
sudo systemctl restart claw-node   # Restart
sudo systemctl stop claw-node      # Stop
```

### Linux (nohup) — Quick & Simple

```bash
nohup claw-node start --network testnet > ~/claw-node.log 2>&1 &

# View logs
tail -f ~/claw-node.log

# Stop
pkill claw-node
```

### macOS (launchd)

```bash
cat > ~/Library/LaunchAgents/com.clawlabz.claw-node.plist << 'EOF'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>com.clawlabz.claw-node</string>
    <key>ProgramArguments</key>
    <array>
        <string>/usr/local/bin/claw-node</string>
        <string>start</string>
        <string>--network</string>
        <string>testnet</string>
    </array>
    <key>RunAtLoad</key>
    <true/>
    <key>KeepAlive</key>
    <true/>
    <key>StandardOutPath</key>
    <string>/tmp/claw-node.log</string>
    <key>StandardErrorPath</key>
    <string>/tmp/claw-node.log</string>
</dict>
</plist>
EOF

launchctl load ~/Library/LaunchAgents/com.clawlabz.claw-node.plist

# View logs
tail -f /tmp/claw-node.log

# Stop
launchctl unload ~/Library/LaunchAgents/com.clawlabz.claw-node.plist
```

### Windows (Task Scheduler)

```powershell
# Register as a scheduled task that starts on boot
$action = New-ScheduledTaskAction -Execute "$env:USERPROFILE\.clawnetwork\bin\claw-node.exe" -Argument "start --network testnet"
$trigger = New-ScheduledTaskTrigger -AtStartup
$settings = New-ScheduledTaskSettingsSet -RestartInterval (New-TimeSpan -Seconds 10) -RestartCount 999
Register-ScheduledTask -TaskName "ClawNode" -Action $action -Trigger $trigger -Settings $settings -RunLevel Highest

# Start now
Start-ScheduledTask -TaskName "ClawNode"

# Check status
Get-ScheduledTask -TaskName "ClawNode" | Select State

# Stop
Stop-ScheduledTask -TaskName "ClawNode"

# Remove
Unregister-ScheduledTask -TaskName "ClawNode" -Confirm:$false
```

### Docker (all platforms)

```bash
docker run -d \
  --name claw-node \
  --restart unless-stopped \
  -p 9710:9710 \
  -p 9711:9711 \
  -v claw-data:/data \
  ghcr.io/clawlabz/claw-node:latest \
  start --data-dir /data --network testnet

# View logs
docker logs -f claw-node

# Stop / Start
docker stop claw-node
docker start claw-node
```

## Monitoring

```bash
# Health check (returns JSON with status, height, peer count)
curl http://localhost:9710/health

# Prometheus metrics (for Grafana dashboards)
curl http://localhost:9710/metrics

# Check block height
curl -s -H "Content-Type: application/json" http://localhost:9710 \
  -d '{"jsonrpc":"2.0","method":"clw_blockNumber","params":[],"id":1}'
```

## Troubleshooting

### Linux: `GLIBC_X.XX not found`

The default Linux binary is statically linked (musl) since v0.1.1 and should work on any distro. If you're using an older release:

```bash
# Check your glibc version
ldd --version

# Solution: upgrade to v0.1.1+ or use Docker
docker run -d -p 9710:9710 -p 9711:9711 ghcr.io/clawlabz/claw-node:latest
```

### Linux: `claw-node: command not found` after install

Some distros (Alibaba Linux, Amazon Linux) don't include `/usr/local/bin` in root's PATH:

```bash
export PATH=$PATH:/usr/local/bin
echo 'export PATH=$PATH:/usr/local/bin' >> ~/.bashrc
```

### Alibaba Linux / Amazon Linux: Docker install fails

The official Docker install script (`get.docker.com`) doesn't support these distros. Use:

```bash
# Alibaba Linux / CentOS-compatible
yum install -y yum-utils
yum-config-manager --add-repo https://mirrors.aliyun.com/docker-ce/linux/centos/docker-ce.repo
yum install -y docker-ce docker-ce-cli containerd.io
systemctl start docker && systemctl enable docker
```

Or skip Docker and use the static binary directly (recommended).

### Windows: Architecture detection error

If the PowerShell installer reports wrong architecture, download manually:

```powershell
Invoke-WebRequest -Uri "https://github.com/clawlabz/claw-network/releases/latest/download/claw-node-windows-x86_64.zip" -OutFile "$env:TEMP\claw-node.zip"
Expand-Archive -Path "$env:TEMP\claw-node.zip" -DestinationPath "$env:USERPROFILE\.clawnetwork\bin" -Force
```
