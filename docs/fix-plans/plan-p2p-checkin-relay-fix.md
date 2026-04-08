# P2P Checkin Relay Fix Plan

**Date**: 2026-04-08
**Severity**: HIGH — 本机矿工因 checkin 无法传播到出块节点而被标记为 inactive，持续丢失挖矿收益
**Status**: 审核通过 (缩减版)，执行 Fix 1 + Fix 2

## 1. 问题描述

本机矿工 `openclaw-miner-b68807c3` (addr `455c1cab...`) 在链上状态为 inactive，
连续缺勤 48+ epochs，每 epoch 丢失约 280 CLAW 挖矿奖励。

Dashboard 显示 Online（节点进程正常），Explorer 显示 Inactive（链上矿工状态）。
OpenClaw 插件按时发送 checkin，本地缓存持续增长（`cache_total=514`），但 checkin 从未到达出块节点。

## 2. 根因分析

### 2.1 直接原因：Checkin gossip 无法到达出块节点

本机 peer_count=1，只连着 Mac Mini（mDNS 局域网发现）。
Hetzner bootstrap peer 启动时连上，~10 秒后 `ConnectionClosed`（sync request outbound failure）。
之后再也没有重连 Hetzner。

### 2.2 根本原因：Bootstrap redial 逻辑缺陷

**文件**: `crates/p2p/src/network.rs:343`

```rust
let should_dial = match peer_id {
    Some(pid) => !self.peers.contains(&pid),
    None => self.peers.is_empty(),  // ← BUG
};
```

启动参数中 bootstrap 地址不含 PeerID（`/ip4/178.156.162.162/tcp/9711`）。
`peer_id` 解析为 `None`，`should_dial = self.peers.is_empty()`。

**注意**: `self.peers` 在 mDNS 发现时即被插入（`network.rs:465` `PeerConnected` 事件），
不仅仅在 TCP 连接建立时。因此只要 mDNS 发现了 Mac Mini，`self.peers` 就非空，
redial 永远为 false。准确判断应看 `ConnectionEstablished/Closed` 事件，而非 `peer_count`。

### 2.3 Hetzner 断开的具体原因

Hetzner 日志显示本节点 PeerID 无 connect/disconnect 记录 — 说明连接在 TCP 层断开，
Hetzner 端 gossipsub 未完成握手。可能是 sync request 请求量大导致 yamux stream 超时。
但这不是核心问题 — 只要 redial 正常就能恢复。

### 2.4 PeerID 权威来源

Hetzner mainnet PeerID 来自启动日志（`journalctl -u clawnet-mainnet`）:
```
P2P identity created local_peer_id=12D3KooWGVXR1MTGqQfnxgpguaiKGEtxc8sFYMbkuJkHdfnuHobG
```
密钥文件：`/opt/clawnet-mainnet/.clawnetwork/p2p_key`（持久化，重启不变）。

阿里云 PeerID 待确认（暂不加，只加 Hetzner）。

## 3. 修复方案 (缩减版)

### Fix 1: Bootstrap redial 逻辑修复（必须，claw-node 侧）

**文件**: `crates/p2p/src/network.rs:343`

```rust
// BEFORE:
None => self.peers.is_empty(),
// AFTER:
None => true,
```

**影响**: 无 PeerID 的 bootstrap 地址始终每 30 秒尝试 redial。开销极低。
**风险**: 无。不碰共识，不碰协议。

### Fix 2: Bootstrap 地址统一追加 PeerID（必须，插件侧）

**三处需要改**:
- `index.ts:24-26` — `BOOTSTRAP_PEERS` 常量
- `index.ts:2073` — install.sh 内联 bootstrapPeers
- `index.ts:2156` — install.sh 内联 bootstrapPeers（第二处）

改为包含 PeerID 的完整 multiaddr:
```
/ip4/178.156.162.162/tcp/9711/p2p/12D3KooWGVXR1MTGqQfnxgpguaiKGEtxc8sFYMbkuJkHdfnuHobG
```

这样 Fix 1 的 redial 条件变为 `Some(pid) => !self.peers.contains(&pid)`（更精确，
只在该 bootstrap peer 确实不在连接列表时才重连）。

**风险**: PeerID 硬编码。Hetzner 若重新生成 p2p_key 需同步更新插件。

### 不做的（审核意见）

- **Fix 3 (explicit peers)**: 单独评估后再决定。`add_explicit_peer` 是拓扑变更，不是纯兜底。
  explicit peer 会收到 flood-publish 但不进 mesh (libp2p-gossipsub 0.47.0 行为)。
- **Fix 4 (epoch 容错)**: 从 hotfix 移除。缓存改 ±1 epoch 需要同步改 `receive_block` 里的
  `w.epoch == block_epoch` 校验 (`chain.rs:1091`)，实质是共识层改动，需完整方案。

## 4. 实施计划

| 步骤 | 内容 | 改动 |
|------|------|------|
| 1 | Fix 1: `network.rs:343` 改 `None => true` | 1 行 |
| 2 | Fix 2: 插件 3 处 bootstrap 加 PeerID | 3 处字符串 |
| 3 | 补 redial regression test | 1 个 test |
| 4 | CI 构建 claw-node v0.5.6 | tag push |
| 5 | **Canary**: 仅升级本机矿工节点 | 不做全网升级 |
| 6 | 观察 2-3 个 epoch (~15 min) 确认 checkin 到达 Hetzner | 验证 |
| 7 | 确认无问题后再推全网 | 后续 |

## 5. 验证方法

1. 本机 OpenClaw 节点重启后日志出现:
   `Peer connected peer_id=12D3KooWGVXR1MTGqQfnxgpguaiKGEtxc8sFYMbkuJkHdfnuHobG`
2. Hetzner 日志出现: `Gossip checkin accepted miner=455c1cab`（或 `checkins=1` 在区块中）
3. `claw_getMinerInfo("455c1cab...")` 返回 `active=true, consecutive_misses=0`
4. 连续 24h peer_count 不再降到 1

## 6. 回滚方案

Fix 1 + Fix 2 不碰共识/协议，纯 P2P 连接行为。回滚只需发布不含改动的版本。
