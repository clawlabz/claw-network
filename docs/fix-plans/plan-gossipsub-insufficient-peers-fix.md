# Gossipsub InsufficientPeers 修复方案 (v5)

**Date**: 2026-04-11
**Severity**: HIGH — 本机矿工连续 46 epochs 未上链，每 epoch 丢失 ~280 CLAW 挖矿收益
**Version**: claw-node v0.5.7, libp2p 0.54 (libp2p-gossipsub 0.47.0)
**Status**: v5 — ACK 语义修正 + self.peers 修正，代码可直接使用

## 1. 问题描述

本机矿工 `openclaw-miner-b68807c3` (addr `455c1cab...`) 再次掉线。
8h Report 显示 Att=x46（连续错过 46 个 epoch），+8h 收益为 0。

节点进程正常运行（PID 75855，4 月 9 日启动），RPC health 正常（height 429424, v0.5.7）。
矿工 checkin 在本地执行成功并缓存（`cache_total=1349`），但链上全部丢失。

**这是第三次** 因 P2P 问题导致同一个矿工掉线。

## 2. 根因分析

### 2.1 publish() 的完整逻辑（libp2p-gossipsub 0.47.0 源码验证）

**文件**: `~/.cargo/registry/src/index.crates.io-*/libp2p-gossipsub-0.47.0/src/behaviour.rs:624-700`

```rust
let mut recipient_peers = HashSet::new();
if let Some(set) = self.topic_peers.get(&topic_hash) {   // ← 外层守卫
    if self.config.flood_publish() {                       // ← flood模式（当前默认开启）
        recipient_peers.extend(set.iter().filter(|p| {
            self.explicit_peers.contains(*p)
                || !self.score_below_threshold(p, |ts| ts.publish_threshold).0
        }));
    } else {                                               // ← 非flood模式
        // mesh peers / fanout / explicit / floodsub peers
    }
}                                                          // ← 注意：整个block在此结束

if recipient_peers.is_empty() {                            // line 698
    return Err(PublishError::InsufficientPeers);
}
```

**关键结构**: 所有 recipient 构建逻辑（包括 mesh、fanout、explicit、floodsub fallback）
**全部在 `if let Some(set) = self.topic_peers.get(&topic_hash)` 守卫内**。

如果 `topic_peers` 没有 checkin topic 的条目 → 整个构建逻辑被跳过 → `recipient_peers` 为空 → `InsufficientPeers`。

### 2.2 flood_publish 默认值验证

**文件**: `libp2p-gossipsub-0.47.0/src/config.rs:409`

```rust
flood_publish: true,  // default
```

**claw-node 未显式设置** → 使用默认值 `true`。

结论：**当前已在 flood 模式**，v1/v2 方案中 "启用 flood_publish" 的 Fix 完全无效。

### 2.3 disconnect 时的状态清理

**文件**: `behaviour.rs:2873-2932` — `on_connection_closed` (remaining_established == 0)

```rust
// remove from topic_peers
if let Some(peer_list) = self.topic_peers.get_mut(topic) {
    peer_list.remove(&peer_id);     // ← 从 topic_peers 中移除
}
// remove from mesh
if let Some(mesh_peers) = self.mesh.get_mut(topic) {
    mesh_peers.remove(&peer_id);    // ← 从 mesh 中移除
}
// ...
self.peer_topics.remove(&peer_id);  // ← 清空 peer 的所有 topic 记录
```

断开连接时，peer 被从 `topic_peers`、`mesh`、`peer_topics`、`connected_peers` 全面清除。

### 2.4 reconnect 时的状态恢复

**文件**: `behaviour.rs:2751-2818` — `on_connection_established`

```rust
// 1. 添加到 connected_peers，初始 PeerKind::Floodsub
self.connected_peers.entry(peer_id).or_insert(PeerConnections {
    kind: PeerKind::Floodsub,
    connections: vec![],
});

// 2. 初始化空的 peer_topics
self.peer_topics.insert(peer_id, Default::default());

// 3. 发送我们的 SUBSCRIBE 给对方
for topic_hash in self.mesh.clone().into_keys() {
    self.send_message(peer_id, RpcOut::Subscribe(topic_hash));
}
```

注意：**只发出了我们的 SUBSCRIBE，但 topic_peers 不会立即恢复**。
必须等待对方回复 SUBSCRIBE，由 `handle_received_subscriptions()` 处理后才会填充 `topic_peers`。

### 2.5 日志验证的确切故障时间线

```
2026-04-09 13:19:17  Peer connected (Hetzner)      peers=1
                     ... 11 小时正常运行，checkin 成功 ...
2026-04-09 20:00:19  Peer disconnected              peers=0
2026-04-09 20:00:48  Peer reconnected               peers=1  ← 正常恢复
2026-04-09 21:00:02  Peer disconnected              peers=0
2026-04-09 21:00:18  Peer reconnected               peers=1  ← 正常恢复
2026-04-10 00:00:01  Peer disconnected              peers=0
2026-04-10 00:00:18  Peer reconnected               peers=1  ← 正常恢复
2026-04-10 00:30:37  Peer disconnected              peers=0
2026-04-10 00:30:41  InsufficientPeers              ← 4秒后，正常（无peer）
2026-04-10 00:30:49  Peer reconnected               peers=1
2026-04-10 00:31:42  InsufficientPeers              ← 重连53秒后仍然报错！
2026-04-10 00:32:42  InsufficientPeers              ← 持续
...                  940 次 InsufficientPeers        ← 再也没恢复
2026-04-11 05:43:38  最后一条 InsufficientPeers
```

**关键观察**:
- 前三次 disconnect/reconnect（20:00、21:00、00:00）后**恢复正常**
- 第四次 disconnect/reconnect（00:30）后**永远没恢复**
- InsufficientPeers 在 peer 重连 53 秒后仍在报错
- 此后 Hetzner 又断连/重连多次，但都没恢复

### 2.6 根因定位

**直接原因**: `topic_peers` 在 peer 断开时被清空，重连后 Hetzner 的 SUBSCRIBE 回复未到达（或未被正确处理），导致 `topic_peers` 持续为空。

**为什么前三次恢复了但第四次没有**: 可能的原因：
1. **gossipsub 协议协商时序**: 重连后 peer 初始为 `PeerKind::Floodsub`，协议协商后变为 `Gossipsub`。
   在 flood_publish 模式下 Floodsub peers **不是** fallback（Floodsub fallback 在 `else` 分支，
   flood 模式走 `if` 分支只看 `topic_peers`）。如果协议协商完成但 SUBSCRIBE 交换失败，
   peer 既不是 Floodsub（fallback 用不了）也不在 topic_peers（flood 用不了）— **死区**。
2. **yamux stream 超时**: `Sync request outbound failure error=ConnectionClosed` 表明 yamux 层有问题，
   可能影响 gossipsub RPC 传输。
3. **随机性**: 前三次可能恰好在 SUBSCRIBE 交换完成后才尝试 publish；第四次恰好在交换完成前 publish 了，
   之后 SUBSCRIBE 交换因某种原因一直没完成。

**结构性问题**:
- **topic_peers 是 publish 的唯一入口**: 无论 flood/non-flood 模式，`topic_peers` 为空 = InsufficientPeers
- **无恢复机制**: topic_peers 清空后，只能靠被动接收对方 SUBSCRIBE 来恢复。没有主动探测/重试
- **无 fallback**: publish 失败后只打日志，不重试、不走其他通道

### 2.7 flood_publish 模式下 Floodsub peers 不生效（反直觉）

在 **非** flood 模式下（`else` 分支，behaviour.rs:685-694），任何 `PeerKind::Floodsub` 的
`connected_peers` 都会被无条件加入 recipient（不需要在 topic_peers 中）。

但在 **flood** 模式下，只看 `topic_peers` 中的 peers，**不查 connected_peers 中的 Floodsub peers**。

讽刺的是：flood_publish=true 的 "洪泛" 模式反而比 flood_publish=false 的标准模式**更严格**。

## 3. 修复方案

### Fix 1: broadcast_checkin 失败时 fallback 到 request-response 直推（P0，核心修复）

gossipsub 有复杂的内部状态管理，修复 topic_peers 恢复问题需要改 libp2p 内部或增加大量 workaround。
**最可靠的方案是绕过 gossipsub，直接通过 request-response 协议推送 checkin。**

#### 1a. 新增 SyncRequest + SyncResponse variant

**文件**: `crates/p2p/src/protocol.rs:65-95`

```rust
// SyncRequest — 新增 PushMinerCheckin (line 74)
pub enum SyncRequest {
    GetBlocks { from_height: u64, count: u32 },
    GetStatus,
    GetStateSnapshot,
    /// Fallback: push a miner checkin witness when gossipsub publish fails.
    PushMinerCheckin(claw_types::state::MinerCheckinWitness),
}

// SyncResponse — 新增 CheckinAccepted (line 95 后)
pub enum SyncResponse {
    Blocks(Vec<Block>),
    Status { height: u64 },
    StateSnapshot { ... },
    /// ACK for PushMinerCheckin — 纯确认，不触发同步逻辑。
    CheckinAccepted,
}
```

**为什么不能复用 `SyncResponse::Status`**:
发送端收到 SyncResponse 后走 `handle_sync_response()` (chain.rs:1471-1484)。
`Status { height }` 会被当成真实同步状态处理 (chain.rs:1633-1660)：
如果对方 height > 本机 height → 触发 `GetBlocks` / `GetStateSnapshot`。
这不是 ACK 语义，会产生不必要的同步请求。

新增 `CheckinAccepted` variant 后，`handle_sync_response` 对未知 variant 返回 `None`（无 follow-up），不触发任何同步。

#### 1b. broadcast_checkin 添加 fallback

**文件**: `crates/p2p/src/network.rs:272-287`

已验证的 API：
- `send_request` 参数类型：`&PeerId, Vec<u8>`（SyncCodec::Request = Vec<u8>，见 behaviour.rs:17-29）
- `MinerCheckinWitness` 已 derive `Clone, BorshSerialize, BorshDeserialize`（state.rs:308）
- 无 SyncRequestWrapper — 直接序列化为 `Vec<u8>`

```rust
pub fn broadcast_checkin(&mut self, witness: &claw_types::state::MinerCheckinWitness) {
    let msg = GossipMessage::MinerCheckin(witness.clone());
    let bytes = borsh::to_vec(&msg).expect("serialize gossip msg");
    if bytes.len() > protocol::MAX_P2P_MESSAGE_SIZE {
        return;
    }
    match self
        .swarm.behaviour_mut().gossipsub
        .publish(self.checkin_topic.clone(), bytes)
    {
        Ok(_) => {
            tracing::debug!(miner=%hex::encode(&witness.miner[..4]), "Checkin published via gossipsub");
        }
        Err(e) => {
            let tcp_peers: Vec<PeerId> = self.swarm.connected_peers().copied().collect();
            tracing::warn!(
                error=%e, tcp_connected=tcp_peers.len(),
                "Gossipsub publish failed, falling back to direct push"
            );
            // Fallback: push via request-response to TCP-connected peers
            // 注意: self.peers 包含 mDNS discovered 但未 TCP 连接的 peer (network.rs:478)
            // tcp_peers 已在上方 warn! 前用 swarm.connected_peers() 获取
            for peer_id in &tcp_peers {
                let req = protocol::SyncRequest::PushMinerCheckin(witness.clone());
                let req_bytes = borsh::to_vec(&req).expect("serialize sync request");
                self.swarm.behaviour_mut().request_response
                    .send_request(peer_id, req_bytes);
                tracing::info!(%peer_id, "Checkin pushed via request-response fallback");
            }
        }
    }
}
```

#### 1c. 接收侧 handler — chain.rs 事件循环

**文件**: `crates/node/src/chain.rs:1463-1470`

现有代码：
```rust
NetworkEvent::SyncRequest { peer, request, channel, .. } => {
    let response = self.handle_sync_request(&request);
    tracing::debug!(?peer, "Sync request handled, sending response");
    let _ = command_tx.send(P2pCommand::SendSyncResponse {
        channel,
        response,
    });
}
```

改为：
```rust
NetworkEvent::SyncRequest { peer, request, channel, .. } => {
    match request {
        SyncRequest::PushMinerCheckin(witness) => {
            // Same logic as NetworkEvent::MinerCheckin handler (line 1439-1462)
            let mut inner = self.inner.lock().expect("chain state mutex poisoned");
            if inner.state.block_height >= CHECKIN_V3_HEIGHT {
                match Self::validate_checkin_witness(&witness, &inner) {
                    Ok(()) => {
                        tracing::debug!(
                            miner = hex::encode(&witness.miner[..4]),
                            epoch = witness.epoch,
                            "Direct-push checkin accepted into cache"
                        );
                        inner.checkin_cache.insert(witness);
                    }
                    Err(reason) => {
                        tracing::warn!(
                            miner = hex::encode(&witness.miner[..4]),
                            reason = %reason,
                            "Direct-push checkin rejected"
                        );
                    }
                }
            }
            drop(inner);
            // Ack: 专用 CheckinAccepted，不触发同步
            let _ = command_tx.send(P2pCommand::SendSyncResponse {
                channel,
                response: SyncResponse::CheckinAccepted,
            });
        }
        other => {
            let response = self.handle_sync_request(&other);
            tracing::debug!(?peer, "Sync request handled, sending response");
            let _ = command_tx.send(P2pCommand::SendSyncResponse {
                channel,
                response,
            });
        }
    }
}
```

**说明**:
- 不能放在 `handle_sync_request` 中，因为它只有 `&self`（不可变），无法写入 `checkin_cache`
- 直接在事件循环中处理，复用 `validate_checkin_witness` 静态方法
- 响应用专用 `SyncResponse::CheckinAccepted`（不能用 `Status { height }`，会触发同步）
- 响应通过 `P2pCommand::SendSyncResponse` 发送（network.rs:52-56 → send_response）

**handle_sync_response 侧**（chain.rs:1590 左右）需新增：
```rust
SyncResponse::CheckinAccepted => None,  // 纯 ACK，无 follow-up
```
```

**风险评估**:

| 项 | 分析 |
|------|------|
| 协议变更 | 新增 SyncRequest::PushMinerCheckin + SyncResponse::CheckinAccepted |
| 向后兼容 | 旧版本收到新 variant → `try_from_slice` 返回 Err → warn 日志 → 忽略（不 crash） |
| 向前兼容 | 新版本收到旧版的 3 个 request / 3 个 response variant → 正常处理 |
| 部署顺序 | Hetzner（出块节点）先升级 → 能处理 PushMinerCheckin 并返回 CheckinAccepted → 本机升级后 fallback 有效 |
| 共识影响 | 无。走的是现有 `validate_checkin_witness` → `checkin_cache.insert` 流程 |
| ACK 安全 | CheckinAccepted 在 handle_sync_response 中返回 None（无 follow-up），不触发同步 |

#### 1d. Borsh 向后兼容验证（前置）

需要写单元测试确认 **两个方向**（SyncRequest + SyncResponse）：

```rust
#[test]
fn old_sync_request_rejects_new_variant() {
    // Serialize new SyncRequest variant
    let new_req = SyncRequestNew::PushMinerCheckin(dummy_witness());
    let bytes = borsh::to_vec(&new_req).unwrap();
    // Old enum (3 variants) try_from_slice should return Err, not panic
    assert!(SyncRequestOld::try_from_slice(&bytes).is_err());
}

#[test]
fn old_sync_response_rejects_new_variant() {
    // Serialize new SyncResponse variant
    let new_resp = SyncResponseNew::CheckinAccepted;
    let bytes = borsh::to_vec(&new_resp).unwrap();
    // Old enum (3 variants: Blocks/Status/StateSnapshot) should return Err, not panic
    assert!(SyncResponseOld::try_from_slice(&bytes).is_err());
}
```

两个测试覆盖完整的协议变更面：请求侧（PushMinerCheckin）和响应侧（CheckinAccepted）。

### Fix 2: 降低 mesh 参数 + mesh 自愈（P1，辅助）

虽然 `mesh_n_low` 不直接影响 InsufficientPeers，但降低 mesh 参数可以：
- 让 heartbeat 更快把新连接的 peer GRAFT 进 mesh
- 减少 mesh churn（不会因为达不到 mesh_n=6 而持续 GRAFT/PRUNE）

**文件**: `crates/p2p/src/behaviour.rs:101`

```rust
let gossipsub_config = gossipsub::ConfigBuilder::default()
    .heartbeat_interval(Duration::from_secs(1))
    .validation_mode(gossipsub::ValidationMode::Strict)
    .max_transmit_size(MAX_P2P_MESSAGE_SIZE)
    // 适配 <10 节点的小网络
    .mesh_n(3)
    .mesh_n_low(1)
    .mesh_n_high(6)
    .mesh_outbound_min(1)
    .build()
```

**效果**: 间接帮助 topic_peers 更快恢复后 mesh 更快填充，但不解决 topic_peers 为空的核心问题。

### Fix 3: 重连后主动 re-subscribe 刷新 topic_peers（P1，辅助）

在 bootstrap_redial tick 中检测 topic_peers 状态，为空时主动 re-subscribe：

**文件**: `crates/p2p/src/network.rs` — bootstrap_redial handler（约 line 336）

已验证的 API：
- `self.checkin_topic` 类型：`gossipsub::IdentTopic`（network.rs:79）
- `self.checkin_topic.hash()` 返回 `TopicHash`（topic.rs:108）
- `all_peers()` 返回 `impl Iterator<Item = (&PeerId, Vec<&TopicHash>)>`（behaviour.rs:502-507）
- `mesh_peers(&TopicHash)` 返回 `impl Iterator<Item = &PeerId>`（behaviour.rs:489-492）

```rust
_ = bootstrap_redial.tick() => {
    // Existing redial logic...

    // Fix 3: If TCP-connected but no peers in checkin topic, re-subscribe
    // 用 swarm.connected_peers() 判断真实 TCP 连接，与 Fix 1/Fix 4 口径一致
    let tcp_count = self.swarm.connected_peers().count();
    if tcp_count > 0 {
        let checkin_hash = self.checkin_topic.hash();
        let has_topic_peers = self.swarm.behaviour()
            .gossipsub.all_peers()
            .any(|(_, topics)| topics.contains(&&checkin_hash));

        if !has_topic_peers {
            tracing::warn!(
                tcp_connected = tcp_count,
                "No peers subscribed to checkin topic, re-subscribing to trigger exchange"
            );
            let _ = self.swarm.behaviour_mut().gossipsub.unsubscribe(&self.checkin_topic);
            let _ = self.swarm.behaviour_mut().gossipsub.subscribe(&self.checkin_topic);
        }
    }
}
```

**注意**: `all_peers()` 返回 `Vec<&TopicHash>`，所以 `contains()` 需要 `&&TopicHash`。
使用 `swarm.connected_peers()` 而非 `self.peers`，避免 mDNS discovered 但未 TCP 连接的 peer 导致误触发。

**原理**: re-subscribe 触发 `join()` → 发送 SUBSCRIBE 给所有 connected_peers → 对方响应 SUBSCRIBE → 填充 topic_peers。

**风险**: 低。re-subscribe 是幂等操作。

### Fix 4: 诊断日志增强（P0，与 Fix 1 一起）

在 `broadcast_checkin` 中（publish 调用前）增加 gossipsub 内部状态日志：

**文件**: `crates/p2p/src/network.rs:272`（broadcast_checkin 函数开头）

已验证的 API（同 Fix 3）：
- `self.checkin_topic.hash()` → `TopicHash`
- `all_peers()` → `(&PeerId, Vec<&TopicHash>)`
- `mesh_peers(&TopicHash)` → `&PeerId` 迭代器

```rust
// Insert before publish() call
let checkin_hash = self.checkin_topic.hash();
let topic_peer_count = self.swarm.behaviour()
    .gossipsub.all_peers()
    .filter(|(_, topics)| topics.contains(&&checkin_hash))
    .count();
let mesh_count = self.swarm.behaviour()
    .gossipsub.mesh_peers(&checkin_hash).count();
// self.peers 包含 mDNS discovered (可能未 TCP 连接)
// swarm.connected_peers() 是真正的 TCP 连接数
let tcp_connected = self.swarm.connected_peers().count();
tracing::debug!(
    topic_peers = topic_peer_count,
    mesh_peers = mesh_count,
    tcp_connected = tcp_connected,
    mdns_peers = self.peers.len(),
    "Attempting checkin publish"
);
```

## 4. 推荐实施顺序

| 优先级 | Fix | 改动量 | 风险 | 效果 |
|--------|-----|--------|------|------|
| P0 | Fix 1 (request-response fallback) | ~40 行 | 中 | **彻底解决**: 绕过 gossipsub 状态管理 |
| P0 | Fix 4 (诊断日志) | ~10 行 | 无 | 未来排查 gossipsub 问题 |
| P1 | Fix 2 (mesh 参数) | 4 行 | 低 | 辅助: mesh 更适配小网络 |
| P1 | Fix 3 (re-subscribe) | ~12 行 | 低 | 辅助: 主动修复 topic_peers |

**实施策略**:
1. Fix 1d: 先写 borsh 向后兼容测试
2. Fix 1a-1c + Fix 4: 一个 commit（核心修复 + 诊断）
3. Fix 2 + Fix 3: 一个 commit（辅助修复）
4. 部署: Hetzner → 本机 → 其他节点

## 5. 版本规划

- **claw-node v0.5.8**: Fix 1 + Fix 2 + Fix 3 + Fix 4
- **协议变更**: SyncRequest 新增 PushMinerCheckin variant
- **部署顺序**: Hetzner 先升级（接收端）→ 本机升级（发送端）→ Mac Mini → Win11 → 阿里云
- **插件**: 无需改动

## 6. 验证方法

1. Borsh 向后兼容测试通过
2. 本机重启后，checkin 通过 request-response fallback 到达 Hetzner
3. Hetzner 日志: `Gossip checkin accepted into cache miner=455c1cab`
4. 链上 `active=true, consecutive_misses=0`
5. 人为 kill Hetzner gossipsub（只保留 request-response）→ 确认 fallback 独立工作
6. 连续 48h 运行，包含多次 peer disconnect/reconnect → checkin 不中断

## 7. 回滚方案

- Fix 2 + Fix 3 + Fix 4: 纯行为/日志改动，revert 即可
- Fix 1: 协议变更，但向后兼容。回滚后 fallback 失效，退化为 gossipsub only。
  如果 gossipsub 仍然失败，需要手动重启节点恢复 topic_peers

## 8. v1 → v2 → v3 → v4 → v5 修正记录

| 版本 | 说法 | 修正 | 依据 |
|------|------|------|------|
| v1 | `mesh_n_low` 降低可防止 InsufficientPeers | **错误**。publish 不检查 mesh_n_low | behaviour.rs:698 只查 recipient_peers.is_empty() |
| v1 | explicit peers 可作 flood fallback | **条件不足**。非 flood 模式需在 topic_peers 中 | behaviour.rs:678-682 |
| v2 | 启用 `flood_publish(true)` 可解决 | **无效**。默认已是 true | config.rs:409 |
| v2 | flood 模式绕过 mesh 依赖 | **不完整**。flood 模式仍受 topic_peers 守卫限制 | behaviour.rs:625 外层 if let |
| v2 | Floodsub peers 是 flood 模式的后备 | **错误**。Floodsub fallback 在 else 分支，flood 模式不走 | behaviour.rs:685-694 在 else 内 |
| v3 | **根因是 topic_peers 重连后未恢复** | ✅ | 日志: 重连 53s 后仍 InsufficientPeers |
| v3 | 代码中使用 `SyncRequestWrapper` | **不存在**。send_request 直接接受 `Vec<u8>` | behaviour.rs:17-29 SyncCodec::Request = Vec<u8> |
| v3 | `SyncResponse::Status { height, version }` | **错误**。只有 `{ height }` | protocol.rs:82 |
| v3 | handler 放在 `handle_sync_request` | **不可行**。该函数只有 `&self`，无法写 cache | chain.rs:1537 签名 |
| v3 | `self.checkin_topic_hash` 字段 | **不存在**。需用 `self.checkin_topic.hash()` | network.rs:79 类型 IdentTopic, topic.rs:108 hash() |
| v4 | ACK 复用 `Status { height }` | **错误**。会触发 handle_sync_response 同步逻辑 | chain.rs:1633 — Status 触发 GetBlocks |
| v4 | fallback 用 `self.peers` 遍历 | **不精确**。含 mDNS discovered 未 TCP 连接的 peer | network.rs:478 mDNS 插入 vs :493 TCP 建立 |
| v5 | 新增 `SyncResponse::CheckinAccepted` + 用 `swarm.connected_peers()` | ✅ 可直接使用 | 验证完毕 |

## 9. 历史关联

| 版本 | 问题 | 根因 | 修复 |
|------|------|------|------|
| v0.5.5 | 本机矿工 inactive 48 epochs | bootstrap 不重连 Hetzner | v0.5.6: redial `None => true` + PeerID |
| v0.5.6 | 本机矿工 checkin 不上链 | gossip epoch mismatch | v0.5.7: epoch ±1 容错 |
| v0.5.7 | 本机矿工 inactive 46 epochs | 重连后 topic_peers 未恢复 + 无 fallback | **本方案**: request-response fallback + 诊断日志 |

**模式**: 本机是唯一的非验证者 light node 矿工，P2P 条件最差。
验证者矿工（Mac Mini / Win11）checkin 走本地 RPC 不经 gossip，不受影响。
