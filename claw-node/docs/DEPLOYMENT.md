# ClawNetwork Node Deployment Guide

## Architecture Overview

```
                    ┌─────────────────────────────────────────┐
                    │  Hetzner CCX13 (178.156.162.162)        │
                    │  Ubuntu 24.04 / Ashburn, VA             │
                    │                                         │
   rpc.clawlabz.xyz│  ┌──────────┐     ┌──────────────────┐  │
  ─────────────────►│  │  Nginx   │────►│ clawnet-mainnet  │  │
                    │  │  :443    │     │ user: clawnet-m  │  │
                    │  │  SSL     │     │ RPC:  :9710      │  │
testnet-rpc.        │  │  CORS    │     │ P2P:  :9711      │  │
  clawlabz.xyz      │  │          │     │ Data: /opt/...   │  │
  ─────────────────►│  │          │     └──────────────────┘  │
                    │  │          │     ┌──────────────────┐  │
                    │  │          │────►│ clawnet-testnet  │  │
                    │  └──────────┘     │ user: clawnet-t  │  │
                    │                   │ RPC:  :9720      │  │
                    │  ┌──────────┐     │ P2P:  :9721      │  │
                    │  │   ufw    │     │ Data: /opt/...   │  │
                    │  │ 22,80,   │     └──────────────────┘  │
                    │  │ 443,9711 │                           │
                    │  │ 9721     │                           │
                    │  └──────────┘                           │
                    └─────────────────────────────────────────┘
```

## Server Info

| Item | Value |
|------|-------|
| Provider | Hetzner Cloud |
| Model | CCX13 (2 vCPU / 8GB RAM / 80GB SSD) |
| Location | Ashburn, VA (us-east) |
| IP | 178.156.162.162 |
| OS | Ubuntu 24.04 |
| Cost | ~$14.49/mo |

## Process Isolation

Each network runs under a **separate Linux user** with its own home directory. They share nothing.

| | Mainnet | Testnet |
|---|---------|---------|
| User | `clawnet-mainnet` | `clawnet-testnet` |
| Home | `/opt/clawnet-mainnet/` | `/opt/clawnet-testnet/` |
| Binary | `/opt/clawnet-mainnet/bin/claw-node` | `/opt/clawnet-testnet/bin/claw-node` |
| Data | `/opt/clawnet-mainnet/.clawnetwork/` | `/opt/clawnet-testnet/.clawnetwork/` |
| RPC Port | 9710 | 9720 |
| P2P Port | 9711 | 9721 |
| Service | `clawnet-mainnet.service` | `clawnet-testnet.service` |
| Public URL | `https://rpc.clawlabz.xyz` | `https://testnet-rpc.clawlabz.xyz` |

## Validator Addresses

```
Mainnet: ffa28f7c6469ab7490ce540a0e49aa64bc77b4dc5bb2a83b17ddd10a9c8ea62e
Testnet: 53b66a4de468a72b1f3dd995b080733d3b97c83f6a5c6df962ae76d2c91a6c63
```

## Security

### systemd Hardening
- `Restart=always` + `RestartSec=5` — auto-restart on crash
- `StartLimitBurst=5` + `StartLimitIntervalSec=60` — max 5 restarts per minute
- `MemoryMax=2G` — prevent OOM from affecting other services
- `CPUQuota=80%` — prevent CPU starvation
- `NoNewPrivileges=true` — cannot gain new privileges
- `ProtectSystem=strict` — filesystem is read-only except allowed paths
- `ProtectHome=true` — cannot access /home
- `PrivateTmp=true` — isolated /tmp
- `PrivateDevices=true` — no access to physical devices
- `ProtectKernelTunables/Modules/ControlGroups=true` — kernel protection

### Firewall (ufw)
```
22/tcp    — SSH
80/tcp    — HTTP (certbot renewal)
443/tcp   — HTTPS (Nginx reverse proxy)
9711/tcp  — Mainnet P2P
9721/tcp  — Testnet P2P
```
RPC ports (9710, 9720) are **NOT** exposed to the internet — accessible only via Nginx.

### SSL
- Let's Encrypt via certbot
- Auto-renewal via systemd timer
- HTTP → HTTPS redirect enabled

### Nginx
- Reverse proxy with CORS headers (`Access-Control-Allow-Origin: *`)
- Proxy timeout: 30s read, 5s connect
- Server tokens hidden

## Operations

### SSH Access
```bash
ssh root@178.156.162.162
```

### Service Management
```bash
# Check status
systemctl status clawnet-mainnet
systemctl status clawnet-testnet

# Restart a node
systemctl restart clawnet-mainnet
systemctl restart clawnet-testnet

# Stop a node
systemctl stop clawnet-mainnet

# Start a node
systemctl start clawnet-mainnet

# Enable/disable auto-start on boot
systemctl enable clawnet-mainnet
systemctl disable clawnet-mainnet
```

### Logs
```bash
# Follow live logs
journalctl -u clawnet-mainnet -f
journalctl -u clawnet-testnet -f

# Last 100 lines
journalctl -u clawnet-mainnet -n 100 --no-pager

# Logs since last boot
journalctl -u clawnet-mainnet -b

# Logs from specific time
journalctl -u clawnet-mainnet --since "2026-03-17 00:00:00"

# Check for errors only
journalctl -u clawnet-mainnet -p err --no-pager
```

### Health Checks
```bash
# Via HTTPS (from anywhere)
curl https://rpc.clawlabz.xyz/health
curl https://testnet-rpc.clawlabz.xyz/health

# Via localhost (from server)
curl http://localhost:9710/health
curl http://localhost:9720/health

# Prometheus metrics
curl http://localhost:9710/metrics
```

### Upgrade Node
Use the automated script:
```bash
# From local machine (downloads CI binary, uploads, restarts both nodes)
./scripts/deploy-hetzner.sh

# Upgrade only mainnet
./scripts/deploy-hetzner.sh --mainnet-only

# Upgrade only testnet
./scripts/deploy-hetzner.sh --testnet-only

# Skip version bump (redeploy current)
./scripts/deploy-hetzner.sh --skip-bump
```

Or manually:
```bash
# 1. Download latest release
gh release download v0.1.22 --pattern "claw-node-linux-x86_64.tar.gz" --dir /tmp

# 2. Upload to server
scp /tmp/claw-node-linux-x86_64.tar.gz root@178.156.162.162:/tmp/

# 3. SSH and upgrade
ssh root@178.156.162.162 << 'EOF'
  systemctl stop clawnet-mainnet clawnet-testnet

  tar xzf /tmp/claw-node-linux-x86_64.tar.gz -C /opt/clawnet-mainnet/bin/
  cp /opt/clawnet-mainnet/bin/claw-node /opt/clawnet-testnet/bin/claw-node
  chown clawnet-mainnet:clawnet-mainnet /opt/clawnet-mainnet/bin/claw-node
  chown clawnet-testnet:clawnet-testnet /opt/clawnet-testnet/bin/claw-node

  systemctl start clawnet-mainnet clawnet-testnet
  sleep 3
  curl -s http://localhost:9710/health | jq .
  curl -s http://localhost:9720/health | jq .
EOF
```

### Data Migration
To migrate to a new server, only `chain.redb` and `key.json` need to be copied:
```bash
# From old server to new server
scp root@OLD_IP:/opt/clawnet-mainnet/.clawnetwork/chain.redb \
    root@NEW_IP:/opt/clawnet-mainnet/.clawnetwork/chain.redb

scp root@OLD_IP:/opt/clawnet-mainnet/.clawnetwork/key.json \
    root@NEW_IP:/opt/clawnet-mainnet/.clawnetwork/key.json

# Same for testnet
scp root@OLD_IP:/opt/clawnet-testnet/.clawnetwork/chain.redb \
    root@NEW_IP:/opt/clawnet-testnet/.clawnetwork/chain.redb

# Then restart on new server
ssh root@NEW_IP 'systemctl restart clawnet-mainnet clawnet-testnet'
```

### SSL Certificate
```bash
# Check certificate status
ssh root@178.156.162.162 'certbot certificates'

# Force renewal
ssh root@178.156.162.162 'certbot renew --force-renewal'

# Auto-renewal is handled by systemd timer:
ssh root@178.156.162.162 'systemctl list-timers certbot'
```

### Disk Usage
```bash
ssh root@178.156.162.162 'du -sh /opt/clawnet-mainnet/.clawnetwork/ /opt/clawnet-testnet/.clawnetwork/'
```

### Backup
```bash
# Backup chain data
ssh root@178.156.162.162 '
  systemctl stop clawnet-mainnet
  cp /opt/clawnet-mainnet/.clawnetwork/chain.redb /opt/clawnet-mainnet/.clawnetwork/chain.redb.bak
  systemctl start clawnet-mainnet
'
```

## Monitoring Checklist

| Check | Command | Expected |
|-------|---------|----------|
| Service running | `systemctl is-active clawnet-mainnet` | `active` |
| Health endpoint | `curl https://rpc.clawlabz.xyz/health` | `{"status":"ok",...}` |
| Block height increasing | Compare `height` in health over time | Increasing every 3s |
| Peer count | `peer_count` in health response | ≥1 |
| Disk space | `df -h /opt/` | <80% used |
| Memory | `free -h` | <80% used |
| SSL valid | `certbot certificates` | Not expired |

## Troubleshooting

### Node won't start
```bash
# Check logs
journalctl -u clawnet-mainnet -n 50 --no-pager

# Common issues:
# - "Read-only file system" → ReadWritePaths in systemd needs updating
# - "Address already in use" → another process on the port
# - "database error" → corrupted chain.redb, restore from backup
```

### Node stuck (not producing blocks)
```bash
# Check if process is running
ps aux | grep claw-node

# Check last block age
curl -s http://localhost:9710/health | jq .last_block_age_secs
# If >60s on single-node, the node may need restart

systemctl restart clawnet-mainnet
```

### Nginx errors
```bash
# Test config
nginx -t

# Check error log
tail -50 /var/log/nginx/error.log

# Reload after config change
systemctl reload nginx
```

## File Locations

| File | Path |
|------|------|
| Mainnet binary | `/opt/clawnet-mainnet/bin/claw-node` |
| Mainnet data | `/opt/clawnet-mainnet/.clawnetwork/chain.redb` |
| Mainnet key | `/opt/clawnet-mainnet/.clawnetwork/key.json` |
| Mainnet config | `/opt/clawnet-mainnet/.clawnetwork/config.toml` |
| Testnet binary | `/opt/clawnet-testnet/bin/claw-node` |
| Testnet data | `/opt/clawnet-testnet/.clawnetwork/chain.redb` |
| Mainnet systemd | `/etc/systemd/system/clawnet-mainnet.service` |
| Testnet systemd | `/etc/systemd/system/clawnet-testnet.service` |
| Nginx mainnet | `/etc/nginx/sites-available/clawnet-mainnet` |
| Nginx testnet | `/etc/nginx/sites-available/clawnet-testnet` |
| SSL cert | `/etc/letsencrypt/live/rpc.clawlabz.xyz/` |
| Log rotation | `/etc/logrotate.d/clawnet` |

## Legacy: Alibaba Cloud Node

The original testnet runs on Alibaba Cloud (`39.102.144.231:9710`) using `devnet` mode.
Deploy script: `./scripts/deploy-alibaba.sh`. This node will be decommissioned after
Hetzner testnet is fully validated.
