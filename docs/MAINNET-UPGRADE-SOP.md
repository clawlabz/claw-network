# ClawNetwork Mainnet Upgrade SOP

> Status: Recommended production runbook
> Last reviewed: 2026-03-26
> Scope: ClawNetwork mainnet validator / RPC node binary upgrades

## 1. Purpose

This document defines the minimum safe procedure for upgrading ClawNetwork mainnet nodes.

It is intentionally stricter than the current shell scripts. The current scripts are helpers, not the source of truth. If a script conflicts with this SOP, follow the SOP.

## 2. Principles

1. Production upgrades are fail-closed, not convenience-first.
2. Never upgrade all mainnet nodes in parallel.
3. Never treat "`/health` returns a height" as sufficient proof of a good upgrade.
4. Never continue to the next node until the current node passes continuity checks.
5. Never delete `chain.redb` on mainnet during an upgrade.
6. A backup that has not been load-tested is not a valid backup.

## 3. Current Gaps You Must Account For

These are important because the current codebase does not fully enforce them for you:

1. Mainnet/testnet startup is now fail-closed on empty storage unless `--allow-genesis` is explicitly passed. Keep it that way; production upgrade paths must not bypass this guard casually.
2. `/health` currently exposes `version`, `height`, `epoch`, `peer_count`, and block age, but not `chain_id` or `genesis_hash`.
3. Current deploy scripts perform backup load tests, but they do not fail closed consistently when verification fails.
4. `deploy-all.sh` is not yet safe enough to be the only control plane for mainnet because the Mac Mini branch still bypasses the Linux backup and rollback flow.
5. Public `claw-node/scripts/deploy-alibaba.sh` is not an authoritative mainnet runbook and must not be treated as one.

## 4. When This SOP Applies

Use this SOP for:

- binary version upgrades
- hotfix releases
- release candidate promotion to mainnet
- emergency rollback to a previous binary

Do not use this SOP for:

- chain reset events
- genesis replacement
- data recovery experiments on production nodes

## 5. Hard Bans

The following are prohibited during a mainnet upgrade:

- `rm ~/.clawnetwork/chain.redb`
- parallel rollout to all mainnet nodes
- upgrading the last known-good mainnet node before another upgraded node is proven healthy
- `pkill -9` / `kill -9` as a first step
- continuing after a backup verification failure
- continuing after a post-upgrade height reset to `0`
- assuming a node is healthy because the process started

## 6. Required Inputs Before Starting

Prepare these before touching any node:

- target release version and release artifact
- previous stable release artifact for rollback
- maintenance window owner
- node inventory with hostnames, SSH method, service names, data dirs, RPC ports
- explicit upgrade order
- disk free space confirmation on every target node
- a separate untouched mainnet node to compare height against

Record this baseline table before the first node:

| Node | Service | Version | Height | Epoch | Peer Count | Data Dir | Binary Path |
|---|---|---|---|---|---|---|---|
| Example | `clawnet-mainnet` | `0.4.x` | `12345` | `67` | `8` | `/opt/clawnet-mainnet/.clawnetwork` | `/opt/clawnet-mainnet/bin/claw-node` |

Minimum commands to capture baseline:

```bash
curl -sf http://127.0.0.1:9710/health
/opt/clawnet-mainnet/bin/claw-node --version
systemctl status clawnet-mainnet --no-pager
df -h
```

## 7. Recommended Upgrade Order

For mainnet, use a rolling order:

1. non-primary / non-public-critical node first
2. second follower node
3. primary public node last

If a node also runs testnet on the same host, upgrade testnet first, then mainnet.

Do not start with the node currently serving as the primary public RPC unless there is no alternative.

## 8. Standard Per-Node Procedure

Repeat this entire section for exactly one node at a time.

### 8.1 Pre-checks

1. Confirm at least one other mainnet node is healthy and not being modified.
2. Record pre-upgrade `version`, `height`, `epoch`, `peer_count`.
3. Confirm previous rollback binary is available locally or on the host.
4. Confirm enough disk space for at least one full DB copy plus artifact staging.

### 8.2 Stop Cleanly

Preferred:

```bash
systemctl stop clawnet-mainnet
sleep 3
pgrep -f clawnet-mainnet && { echo "ABORT: process still running"; exit 1; }
```

If the process does not exit, investigate first. Only escalate to force-kill if:

- you already captured the situation in logs
- the service is truly wedged
- the operator explicitly accepts the higher risk

### 8.3 Backup After Stop

Create a timestamped backup only after the process has fully exited.

```bash
DB=/opt/clawnet-mainnet/.clawnetwork/chain.redb
BACKUP=/tmp/chain-clawnet-mainnet-backup-$(date +%s).redb
cp "$DB" "$BACKUP"
ls -lh "$BACKUP"
```

Optional quick sanity only:

```bash
xxd -l4 -p "$BACKUP"
```

Do not confuse a valid magic header with a valid backup. This is only a coarse check.

### 8.4 Mandatory Backup Load Test

Before deploying the new binary, verify the backup can be loaded in isolation.

```bash
VERIFY_DIR=/tmp/clawnet-backup-verify
rm -rf "$VERIFY_DIR"
mkdir -p "$VERIFY_DIR"
cp "$BACKUP" "$VERIFY_DIR/chain.redb"
cp /opt/clawnet-mainnet/.clawnetwork/key.json "$VERIFY_DIR/"
cp /opt/clawnet-mainnet/.clawnetwork/config.toml "$VERIFY_DIR/" 2>/dev/null || true

/opt/clawnet-mainnet/bin/claw-node start \
  --network mainnet \
  --single \
  --data-dir "$VERIFY_DIR" \
  --rpc-port 19999 \
  --p2p-port 19998 \
  > /tmp/clawnet-backup-verify.log 2>&1 &

VERIFY_PID=$!
sleep 8
curl -sf http://127.0.0.1:19999/health
kill "$VERIFY_PID" || true
wait "$VERIFY_PID" 2>/dev/null || true
rm -rf "$VERIFY_DIR"
```

Required result:

- process starts
- `/health` returns structured JSON
- `height` is greater than `0`

If backup verification fails, stop the upgrade and do not continue.

### 8.5 Install New Binary

Replace the binary only after the backup passes:

```bash
cp /tmp/claw-node-new /opt/clawnet-mainnet/bin/claw-node
chmod +x /opt/clawnet-mainnet/bin/claw-node
chown clawnet-mainnet:clawnet-mainnet /opt/clawnet-mainnet/bin/claw-node
/opt/clawnet-mainnet/bin/claw-node --version
```

### 8.6 Start Node

```bash
systemctl start clawnet-mainnet
sleep 8
```

### 8.7 Post-Start Acceptance Gates

All of the following must pass before moving to the next node:

1. Service is active:

```bash
systemctl status clawnet-mainnet --no-pager
```

2. Health endpoint returns JSON:

```bash
curl -sf http://127.0.0.1:9710/health
```

3. Reported version equals target release.
4. Height is not `0`.
5. Height has not moved backwards relative to the pre-upgrade baseline except for expected short lag.
6. Within a short catch-up window, the node rejoins cluster progress and approaches the untouched comparison node.
7. `peer_count` is reasonable for that node's normal role.
8. Logs do not show snapshot mismatch, DB corruption, or silent re-init behavior.

### 8.8 Required Log Checks

Immediately inspect recent logs:

```bash
journalctl -u clawnet-mainnet -n 100 --no-pager
```

Treat any of the following as a hard failure:

- `Created genesis block from config`
- `State snapshot mismatch`
- repeated startup loops
- height pinned at `0`
- obvious chain discontinuity compared with other nodes

## 9. Rollback Procedure

Rollback is required if any post-start acceptance gate fails.

### 9.1 Binary Rollback

Keep the previous stable binary available before the upgrade starts.

### 9.2 Data Rollback

If the node touched or replaced data incorrectly, restore the verified backup:

```bash
systemctl stop clawnet-mainnet
cp "$BACKUP" /opt/clawnet-mainnet/.clawnetwork/chain.redb
cp /tmp/claw-node-prev /opt/clawnet-mainnet/bin/claw-node
chmod +x /opt/clawnet-mainnet/bin/claw-node
chown clawnet-mainnet:clawnet-mainnet /opt/clawnet-mainnet/bin/claw-node
systemctl start clawnet-mainnet
```

After rollback, rerun the same health and continuity checks.

## 10. Cluster-Level Completion Criteria

The rollout is complete only when all upgraded nodes satisfy:

- target version matches on every node
- no node reset to height `0`
- all nodes converge near the same tip
- public RPC endpoints match expected network behavior
- logs show normal block production / sync
- backups created during the rollout are retained for the retention window

## 11. Minimum Operational Improvements Still Needed

This SOP works around current platform gaps. It does not replace fixing them.

The following should be implemented in code and tooling:

1. `/health` should expose at least `chain_id` and `genesis_hash`.
2. Deploy scripts should abort on backup load verification failure instead of logging a warning and continuing.
3. `deploy-all.sh` Mac Mini flow should be upgraded to match Linux stop-backup-verify-start semantics.
4. Release automation should compare pre-upgrade and post-upgrade continuity, not just check for a `height` field.
5. Production rollout paths should avoid `--allow-genesis` entirely unless the operator is intentionally initializing a brand-new network.

## 12. Comparison With Mainstream Chains

This SOP is intentionally aligned with mainstream manual upgrade practices:

- Ethereum/Geth style manual upgrades: stop, replace binary, restart, reuse existing DB
- Cosmos validator operations: staged rollout, upgrade gating, rollback readiness

Where ClawNetwork still lags mainstream production maturity:

- backup verification is not yet fail-closed across all deploy paths
- no chain continuity check in health endpoint
- Mac Mini rollout flow is weaker than Linux rollout flow
- too much operator judgment still lives outside the product

## 13. Operator Checklist

Use this as the final short checklist during a real change window:

- [ ] another mainnet node remains untouched and healthy
- [ ] pre-upgrade baseline captured
- [ ] old binary ready for rollback
- [ ] target node stopped cleanly
- [ ] backup created after stop
- [ ] backup load-tested successfully
- [ ] new binary installed
- [ ] post-start version correct
- [ ] post-start height not zero
- [ ] post-start height converges with untouched node
- [ ] logs clean
- [ ] only then proceed to next node
