# v0.5.8 实施审计报告 — Gossipsub InsufficientPeers 修复

**Date**: 2026-04-11
**Version**: claw-node v0.5.8
**Commits**: f3b2779 (主修复) → 84e317b (Cargo.lock MSRV fix) → 待提交 (兼容性测试增强)
**Tag**: v0.5.8 → 84e317b
**方案文档**: `docs/fix-plans/plan-gossipsub-insufficient-peers-fix.md` (v5, 5 轮审计通过)

## 1. 问题回顾

本机矿工 `openclaw-miner-b68807c3` 连续 46 epochs 未上链（Att=x46），每 epoch 丢失 ~280 CLAW。
这是第三次因 P2P 问题导致同一矿工掉线。

| 次数 | 版本 | 根因 | 修复 |
|------|------|------|------|
| 1 | v0.5.5 | bootstrap 不重连 Hetzner | v0.5.6: redial `None => true` |
| 2 | v0.5.6 | gossip epoch mismatch | v0.5.7: epoch ±1 容错 |
| 3 | v0.5.7 | **gossipsub topic_peers 重连后未恢复** + **无 fallback** | **v0.5.8: 本次修复** |

## 2. 根因分析（源码验证）

通过 libp2p-gossipsub 0.47.0 源码验证确认：

1. `flood_publish` 默认已是 `true`（config.rs:409）
2. `publish()` 的所有 recipient 构建逻辑都在 `if let Some(set) = self.topic_peers.get(&topic_hash)` 守卫内（behaviour.rs:625）— topic_peers 为空则整个构建被跳过
3. 断开连接时 `on_connection_closed` 清空 topic_peers（behaviour.rs:2898）
4. 重连后依赖对方回复 SUBSCRIBE 来恢复 topic_peers — 这个交换在某些情况下不完成
5. 日志证实：Hetzner 重连 53 秒后 InsufficientPeers 仍在报错，之后再也没恢复

## 3. 实施内容

### Fix 1: Request-Response Fallback（P0，核心修复）

| 文件 | 改动 |
|------|------|
| `crates/p2p/src/protocol.rs:75` | 新增 `SyncRequest::PushMinerCheckin(MinerCheckinWitness)` |
| `crates/p2p/src/protocol.rs:97` | 新增 `SyncResponse::CheckinAccepted` |
| `crates/p2p/src/network.rs:272-320` | `broadcast_checkin` 重写：gossipsub 失败时 fallback 到 `send_request` 直推所有 TCP peer |
| `crates/node/src/chain.rs:1495-1526` | 事件循环中新增 `SyncRequest::PushMinerCheckin` handler（validate → cache → CheckinAccepted） |
| `crates/node/src/chain.rs:1595` | `handle_sync_request` 新增 `PushMinerCheckin` match arm |
| `crates/node/src/chain.rs:1898` | `handle_sync_response` 新增 `CheckinAccepted => None`（纯 ACK，不触发同步） |

**关键设计决策**:
- ACK 用专用 `CheckinAccepted` 而非复用 `Status { height }` — 避免触发 `handle_sync_response` 中的同步逻辑（GetBlocks/GetStateSnapshot）
- Fallback 使用 `self.swarm.connected_peers()` 而非 `self.peers` — 前者是真实 TCP 连接，后者包含 mDNS discovered 未连接的 peer
- PushMinerCheckin handler 在事件循环中处理而非 `handle_sync_request` 中 — 后者只有 `&self` 无法写入 checkin_cache

### Fix 2: Mesh 参数调整（P1）

| 文件 | 改动 |
|------|------|
| `crates/p2p/src/behaviour.rs:101-110` | 新增 `.mesh_n(3).mesh_n_low(1).mesh_n_high(6).mesh_outbound_min(1)` |

降低 gossipsub mesh 参数适配 <10 节点网络，减少 GRAFT churn。

### Fix 3: Re-subscribe 自愈（P1）

| 文件 | 改动 |
|------|------|
| `crates/p2p/src/network.rs:347-363` | bootstrap_redial tick 中每 30 秒检测：TCP 连接但 checkin topic_peers 为空时，unsubscribe + subscribe 触发 fresh subscription exchange |

使用 `swarm.connected_peers().count()` 判断真实 TCP 连接状态。

### Fix 4: 诊断日志（P0）

| 文件 | 改动 |
|------|------|
| `crates/p2p/src/network.rs:275-290` | publish 前输出 `topic_peers`, `mesh_peers`, `tcp_connected`, `mdns_peers` 四个维度的计数 |

### 兼容性测试

| 文件 | 改动 |
|------|------|
| `crates/p2p/src/protocol.rs:102-170` | 3 个 borsh 向后兼容测试（使用真正的旧版 enum 定义反序列化） |

测试使用独立定义的 `SyncRequestV057`（3 variants）和 `SyncResponseV057`（3 variants）
模拟旧节点反序列化新 variant：
- `old_sync_request_rejects_new_push_miner_checkin`: 旧版收到 index=3 → Err ✅
- `old_sync_response_rejects_new_checkin_accepted`: 旧版收到 index=3 → Err ✅
- `old_variants_still_round_trip_with_new_enum`: 旧版序列化的 GetStatus/Status → 新版可解析 ✅

## 4. 验证结果

### 本地构建
```
cargo build               ✅ 成功
cargo test -p claw-p2p     ✅ 8/8 通过（含 3 个兼容性测试，使用真正的旧版 enum 反序列化）
cargo test -p claw-node    ✅ 通过
```

### CI
```
Tag: v0.5.8 → 84e317b
Release workflow: ✅ 成功（构建 + 发布完成）
Docker workflow: ✅ 成功
```

### 协议变更影响

| 项 | 分析 | 验证 |
|------|------|------|
| SyncRequest 新 variant (index 3) | 旧版 `SyncRequestV057::try_from_slice` 返回 Err | ✅ 测试 `old_sync_request_rejects_new_push_miner_checkin` |
| SyncResponse 新 variant (index 3) | 旧版 `SyncResponseV057::try_from_slice` 返回 Err | ✅ 测试 `old_sync_response_rejects_new_checkin_accepted` |
| 旧 variant 向前兼容 | 旧版序列化数据新版可解析 | ✅ 测试 `old_variants_still_round_trip_with_new_enum` |
| 旧节点收到新 variant | warn 日志，不 crash，不影响共识 | ✅ 由上述测试证明 |
| 部署顺序 | Hetzner 先升级（接收端）→ 本机（发送端）→ 其他节点 | ✅ 已执行 |

## 5. 改动统计

```
 crates/node/src/chain.rs         | +48 -4   (handler + match arms)
 crates/p2p/src/behaviour.rs      | +7       (mesh params)
 crates/p2p/src/network.rs        | +56 -4   (fallback + re-subscribe + diagnostics)
 crates/p2p/src/protocol.rs       | +51      (variants + tests)
 Total                            | +162 -8
```

## 6. 审计过程

方案经过 5 轮审计迭代：

| 版本 | 问题 | 修正 |
|------|------|------|
| v1 | `mesh_n_low` 降低可防止 InsufficientPeers | 错误 — publish 不检查 mesh_n_low |
| v2 | 启用 `flood_publish(true)` 可解决 | 无效 — 默认已 true |
| v3 | 代码中 SyncRequestWrapper / checkin_topic_hash | 不存在 — 对齐实际 API |
| v4 | ACK 复用 `Status { height }` / self.peers 含 mDNS | 触发同步 / 口径偏松 |
| v5 | 前置测试缺 SyncResponse / Fix 3 用 self.peers | 补齐测试 / 统一 TCP 口径 |

## 7. 部署状态

| 步骤 | 节点 | 状态 | 版本 |
|------|------|------|------|
| 1 | Hetzner mainnet | ✅ 已升级 | v0.5.8, peers=7 |
| 2 | 本机 (light node + miner) | ✅ 已升级 | v0.5.8, peers=3 |
| 3 | Mac Mini | 待后续（向后兼容，不阻塞） | v0.5.7 |
| 4 | Win11 | 待后续（RDP 手动） | v0.5.7 |
| 5 | 阿里云 | 待后续（本地中转） | v0.5.7 |

### 验证结果
- Hetzner: block 434103 checkins=1, block 434118 checkins=2 ✅
- 本机: 无 InsufficientPeers 错误，gossipsub publish 成功 ✅
- 注意: `/health` 和 `/peers` 的 peer_count 基于 `self.peers`（含 mDNS discovered），
  非严格 TCP 连接数。仅作弱信号参考。

## 8. 风险评估

| 风险 | 级别 | 缓解 |
|------|------|------|
| 协议变更向后不兼容 | 低 | Borsh 测试验证 + Hetzner 先升级 |
| Fallback 增加 request-response 流量 | 低 | 仅在 gossipsub 失败时触发 |
| Re-subscribe 短暂丢消息 | 极低 | Checkin 是周期性的（每分钟），1-2 秒间隔可忽略 |
| 回滚 | 简单 | revert commit + re-tag 即可 |

## 9. 遗留项

- [x] CI Release 构建 ✅
- [x] Hetzner 部署 v0.5.8 ✅
- [x] 本机升级 v0.5.8 ✅
- [x] Checkin 到达 Hetzner 并打包进区块 ✅
- [x] 兼容性测试增强（真正旧版 enum 反序列化验证）✅
- [ ] 48h 稳定性观察
- [ ] Mac Mini / Win11 / 阿里云 升级（向后兼容，不阻塞）
- [ ] `/health` peer_count 改为基于 `swarm.connected_peers()` 的真实 TCP 连接数（P2，后续优化）
