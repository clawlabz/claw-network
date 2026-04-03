# State Root 版本化方案：解决结构体演进与 state_root 兼容性问题

**日期**: 2026-04-03
**状态**: 待审核 (第七轮 Codex 审核后修订)
**关联**: `2026-04-03-mining-heartbeat-redesign.md` (Heartbeat V2)

---

## 问题发现

### 事故经过

Heartbeat V2 (v0.4.21) 部署到 Hetzner 节点后启动失败。日志：

```
INFO  Loaded chain from storage height=224509
WARN  State snapshot mismatch detected — auto-recovering by deleting chain.redb
      block_root=23005b65... computed_root=11d81501...
INFO  Deleted corrupted chain.redb — will re-initialize from genesis
Error: No existing chain data found. Refusing to create a new genesis on mainnet.
```

### 根因分析

1. MinerInfo 结构新增了 4 个 V2 字段（pending_rewards, pending_epoch, epoch_attendance, consecutive_misses）
2. 旧 snapshot 通过 V1→V2 迁移成功加载（MinerInfoV1 → MinerInfo，新字段填默认值）
3. 节点在启动时做完整性校验：重新计算 `state_root` 与区块头中保存的 `block.state_root` 对比
4. `state_root` 计算中 `borsh::to_vec(miner)` 现在序列化了 12 个字段（V2），而旧区块头的 state_root 是用 8 个字段（V1）算的
5. hash 不匹配 → 节点判定 snapshot 损坏 → 自动删除 chain.redb → 无法启动

### 为什么之前的审核没发现

- 第一轮审核：指出 Borsh 兼容性 → 加了 V1→V2 迁移，解决了**反序列化**
- 第二轮审核：指出 WorldState 新字段 → 加了 `#[borsh(skip)]`，解决了 WorldState 层面
- 第三轮审核：发现瞬态字段参与 state_root + fork 归一化缺失
- 第四轮审核（本轮）：发现 epoch_checkins 丢失导致共识分裂 + 边界一次性结算改变货币政策
- **根本遗漏**：只考虑了"能不能读"，没追踪"读完算出的 hash 是否和旧区块一致"

### 系统性问题

当前 `state_root` 的计算方式是直接 `borsh::to_vec(struct)` 然后 blake3 hash。
**任何参与 state_root 的结构体的任何字段变动都会破坏 state_root 兼容性。**
这不是 MinerInfo 独有的问题——未来修改任何链上结构体都会遇到同样的问题。

---

## 行业对标

| 链 | 状态演进方式 | state_root 兼容策略 |
|---|---|---|
| **Ethereum** | 硬分叉高度切换 | fork height 之前用旧计算，之后用新计算，所有节点同步切换 |
| **Cosmos SDK** | protobuf（天然前后兼容）| IAVL trie 存 KV 对，字段变化不影响已有 KV |
| **Substrate** | `storage_version` + `on_runtime_upgrade()` | 指定高度执行迁移，state trie 基于 KV |

**核心共识**: 不是避免改结构体，而是在 fork height 统一切换序列化格式。

---

## 方案设计：版本化 state_root 计算

### 核心思想

```
fork_height 之前: state_root 中结构体使用旧版序列化 → 和旧区块 hash 一致
fork_height 之时: 运行归一化（normalization），确保所有节点状态一致
fork_height 之后: state_root 包含新字段
```

### 对比其他方案

| 方案 | 优点 | 缺点 | 结论 |
|------|------|------|------|
| **A: 版本化 state_root + 归一化**（推荐） | 数据模型干净；可复用；明确切换点 | 需要 serialize_vN + normalization | ✅ 最佳 |
| B: 新字段拆到独立 map + borsh(skip) | 不改 MinerInfo 布局 | 数据模型分裂；不通用 | ❌ 补丁式 |
| C: 全量 KV 化 state_root（Cosmos 式）| 根本解决兼容问题 | 重构量巨大 | ❌ 过度工程 |
| D: 新字段全部 borsh(skip) | 最简单 | 重启丢数据；不参与共识 | ❌ 不安全 |

---

## 具体实现

### 1. MinerInfo 保留 V2 字段，加 borsh_v1 方法

```rust
impl MinerInfo {
    /// Serialize only V1 fields for backward-compatible state_root computation.
    pub fn borsh_v1(&self) -> Vec<u8> {
        let mut buf = Vec::new();
        // 只序列化原始 8 个字段，顺序与 V1 borsh 布局一致
        BorshSerialize::serialize(&self.address, &mut buf).unwrap();
        BorshSerialize::serialize(&self.tier, &mut buf).unwrap();
        BorshSerialize::serialize(&self.name, &mut buf).unwrap();
        BorshSerialize::serialize(&self.registered_at, &mut buf).unwrap();
        BorshSerialize::serialize(&self.last_heartbeat, &mut buf).unwrap();
        BorshSerialize::serialize(&self.ip_prefix, &mut buf).unwrap();
        BorshSerialize::serialize(&self.active, &mut buf).unwrap();
        BorshSerialize::serialize(&self.reputation_bps, &mut buf).unwrap();
        buf
    }
}
```

### 2. state_root() 根据 block_height 切换序列化

```rust
// In WorldState::state_root():

// Miners
for (addr, miner) in &self.miners {
    let mut entry = Vec::new();
    entry.extend_from_slice(b"miner:");
    entry.extend_from_slice(addr);
    if self.block_height >= HEARTBEAT_V2_HEIGHT {
        // V2: full serialization including epoch scoring fields
        entry.extend_from_slice(&borsh::to_vec(miner).unwrap());
    } else {
        // V1: backward-compatible serialization
        entry.extend_from_slice(&miner.borsh_v1());
    }
    leaves.push(*blake3::hash(&entry).as_bytes());
}
```

### 3. epoch_reward_bucket 持久化 + 参与 state_root（第四轮修订 #3）

`epoch_reward_bucket` 持有已从 pool 扣除但尚未分配给矿工的资金。
**必须持久化并参与 state_root**，否则重启后资金凭空消失（供应量不守恒）。

```rust
// WorldState 中:
// epoch_reward_bucket: u128  — 去掉 #[borsh(skip)]，正常持久化
// 加入 WorldStateV1 → V2 迁移（默认值 0）
// state_root: V2_HEIGHT 后参与 merkle 叶子计算
```

**保留 per-block 矿工份额扣除**（`accumulate_mining_reward`），不改为边界一次性结算。
原因：边界一次性结算会改变低余额下的矿工收益分配——validators 逐块扣 pool 后，
矿工只能拿到 epoch 末尾剩余的零头，这是货币政策变更。
Per-block 扣除确保矿工和 validators 在每个块公平竞争 pool 余额。

### 4. epoch_checkins 用 last_heartbeat 替代（第四轮修订 #1）

**问题**：`epoch_checkins` 是 `#[borsh(skip)]`，mid-epoch 重启后丢失。
epoch 边界处理依赖 check-in 决定 pending 确认/作废、attendance 更新、reputation 衰减。
丢失 check-in → 重启节点和正常节点在边界产生不同状态 → **共识分裂**。

**修复**：epoch 边界不使用 `epoch_checkins`，改用 `miner.last_heartbeat`（已持久化在 MinerInfo 中）。

```rust
// Epoch 边界判断矿工是否在【被结算的 epoch】内签到:
// 注意: 边界高度 height 属于新 epoch，被结算的是上一个 epoch。
let settled_epoch = height / MINER_EPOCH_LENGTH - 1;
let settled_epoch_start = settled_epoch * MINER_EPOCH_LENGTH;
let settled_epoch_end = settled_epoch_start + MINER_EPOCH_LENGTH; // == height
let checked_in = miner.last_heartbeat >= settled_epoch_start
              && miner.last_heartbeat < settled_epoch_end;
// 等价简写:
let checked_in = miner.last_heartbeat / MINER_EPOCH_LENGTH == settled_epoch;
```

**为什么不是 `>= epoch_start`（第五轮修订 #2）**：
- epoch 边界块属于新 epoch，`epoch_start = height` 是新 epoch 的起点
- 如果用 `last_heartbeat >= epoch_start`，边界块中包含的心跳会被误判为"本 epoch 签到"
- 而上个 epoch 正常签到的矿工（`last_heartbeat < epoch_start`）会被误判为缺席
- 正确做法：检查 `last_heartbeat` 是否落在**被结算 epoch** 的范围内

`epoch_checkins` 降级为**纯运行时去重缓存**（heartbeat handler 用来防止同 epoch 重复签到），
不参与 epoch 边界的共识决策，不参与 state_root，保持 `#[borsh(skip)]`。

**关键区别**：
- `epoch_checkins`：runtime dedup cache，`#[borsh(skip)]`，丢失无影响
- `miner.last_heartbeat`：共识状态，持久化，epoch 边界用它判断签到

### 4b. V2 心跳准入改为 epoch-number 判断（第七轮修订 #3）

**问题**：当前 V2 心跳准入用 `block_height >= last_heartbeat + 100`（delta 间隔）。
这导致**相位锁定**：如果矿工在 epoch 的第 80 块签到，下次必须等到下个 epoch 的第 80 块。
如果矿工迟到到第 95 块签到，下次可用窗口只剩 5 块。逐渐漂移最终错过 → 误判缺席。

**修复**：改为 epoch-number 判断：只要矿工上次签到不在当前 epoch 就允许。

```rust
// V2 heartbeat handler:
// 旧 (有相位锁定问题):
//   let next_allowed = miner.last_heartbeat + MINER_HEARTBEAT_INTERVAL;
//   if state.block_height < next_allowed { return Err(HeartbeatTooEarly) }

// 新 (epoch-number 判断, 无相位锁定):
let current_epoch = state.block_height / MINER_EPOCH_LENGTH;
let last_epoch = miner.last_heartbeat / MINER_EPOCH_LENGTH;
if last_epoch >= current_epoch {
    return Err(StateError::HeartbeatTooEarly { ... });
}
// epoch_checkins 仅做同 epoch 内去重（重启后丢失不影响共识，
// 因为 last_heartbeat 更新后 last_epoch == current_epoch 也会拒绝）
```

**效果**：
- 矿工在 epoch 内任何时间签到都行（块 0 到块 99）
- 每 epoch 最多签到一次（epoch-number 相同则拒绝）
- 不受上次签到时间的相位影响
- `epoch_checkins` 去重和 `last_epoch >= current_epoch` 判断是双重保障，
  即使 epoch_checkins 重启后丢失，last_heartbeat 判断仍然正确

**客户端间隔（310 秒）仍然适用**：epoch 长 300 秒（100 块 × 3 秒），
客户端每 310 秒尝试一次 → 保证每 epoch 至少有一次机会落在新 epoch 内。

### 5. Fork 时刻强制归一化 V2 字段（第三轮修订 #2）

**问题**：早升级节点 `handle_miner_register` 写入 V2 字段为零；迁移写入 `epoch_attendance=0xFFFF`。
fork 时刻 state_root 切换到 V2 序列化，不同节点的 V2 字段不同 → 共识分裂。

**修复**：`process_miner_epoch_boundary` 在 V2_HEIGHT（归一化块）执行特殊逻辑。

```rust
pub fn process_miner_epoch_boundary(world: &mut WorldState, height: u64) -> Vec<BlockEvent> {
    if height < HEARTBEAT_V2_HEIGHT { return vec![]; }
    if height % MINER_EPOCH_LENGTH != 0 { return vec![]; }
    
    let is_activation = height == HEARTBEAT_V2_HEIGHT;
    
    if is_activation {
        // === ACTIVATION BOUNDARY (第六轮修订 #2 + #3) ===
        // 
        // 1. 归一化所有矿工 V2 字段为零（确保共识一致）
        for miner in world.miners.values_mut() {
            miner.pending_rewards = 0;
            miner.pending_epoch = 0;
            miner.epoch_attendance = 0;
            miner.consecutive_misses = 0;
        }
        
        // 2. 归零 epoch_reward_bucket（fork 块的 accumulate 可能已放入 1 块奖励）
        //    将其退回 pool，确保激活块不产生偏差。
        if world.epoch_reward_bucket > 0 {
            let pool_addr = genesis_address(NODE_INCENTIVE_POOL_INDEX);
            *world.balances.entry(pool_addr).or_insert(0) += world.epoch_reward_bucket;
            world.epoch_reward_bucket = 0;
        }
        
        // 3. 跳过签到判断 / 缺席惩罚 / reputation 衰减
        //    原因：V1 矿工心跳间隔 1000 块，last_heartbeat 可能在 900 块前，
        //    用 100 块窗口判断会误判所有 V1 合规矿工为缺席。
        //    激活块只做归一化 + 初始化，不做结算。
        
        return vec![]; // 无事件
    }
    
    // === 正常 epoch 结算（从第二个 V2 epoch 开始）===
    let settled_epoch = height / MINER_EPOCH_LENGTH - 1;
    // ... 签到判断用 last_heartbeat / MINER_EPOCH_LENGTH == settled_epoch
    // ... pending 确认/作废, attendance 更新, reputation 衰减
}
```

**激活块特殊处理的原因**：
- **bucket 归零**：`accumulate_mining_reward` 在 `process_miner_epoch_boundary` 之前执行，
  fork 块的 1 块奖励已进入 bucket。如果不归零，这笔奖励会被立即结算到 pending，
  而没有矿工签到过 → 全部退回 pool → 多一次无意义的扣除+退回。直接归零更干净。
- **跳过结算**：V1 矿工 `last_heartbeat` 间隔 1000 块，不满足 100 块窗口 → 全员被判缺席。
  这是对合规矿工的不公平惩罚。激活块只初始化，从第二个 epoch 开始用 V2 规则结算。
- **归一化为零**：确定性，不依赖迁移路径的值。矿工从零开始积累 uptime。
  前 12 个 epoch (~1 小时) 所有矿工 uptime < 50% → 暂时无奖励 → 持续在线的自然达标。

### 6. Snapshot 反序列化保留 V1→V2 迁移

```rust
// WorldState::from_snapshot_bytes():
// 1. 尝试 V2 格式（已升级节点写的 snapshot）
// 2. 失败则回退 V1 格式（旧节点的 snapshot），自动迁移
//
// 迁移后 MinerInfo 的 V2 字段值无关紧要——
// 因为 state_root 在 V2_HEIGHT 前只用 borsh_v1()，
// 而 V2_HEIGHT 到达时归一化会覆盖所有 V2 字段。
```

### 7. 统一激活点：所有 V2 逻辑在同一个 epoch 边界激活（第五轮修订 #4）

**问题**：如果心跳/accumulate 在 V2_HEIGHT 激活，state_root 在第一个 epoch 边界才切换，
中间窗口的新状态变更不在 state_root 中 → 重启后无法验证。

**修复**：`HEARTBEAT_V2_HEIGHT` 本身必须是一个 epoch 边界高度。
所有 V2 逻辑在这同一个块**原子激活**。

```rust
// HEARTBEAT_V2_HEIGHT 必须满足: height % MINER_EPOCH_LENGTH == 0
// 例如: 225_100 (225100 % 100 == 0)
pub const HEARTBEAT_V2_HEIGHT: u64 = 225_100;
```

```
到达 HEARTBEAT_V2_HEIGHT (该块同时是 epoch 边界):
  ├─ accumulate_mining_reward: 第一次 V2 bucket 累加（1 块奖励进 bucket）
  ├─ process_miner_epoch_boundary (激活块特殊逻辑):
  │   ├─ 归一化所有矿工 V2 字段为零
  │   ├─ 归零 epoch_reward_bucket（1 块奖励退回 pool）
  │   ├─ 跳过签到判断 / 缺席惩罚 / reputation 衰减
  │   └─ 返回（不做正常结算）
  ├─ state_root 切换到 V2 完整序列化
  ├─ handle_miner_heartbeat 切换到 V2 模式
  └─ 所有节点状态一致 ✓

HEARTBEAT_V2_HEIGHT + 1 ~ + 99 (第一个完整 V2 epoch):
  ├─ 每块: accumulate_mining_reward 累加到 bucket
  ├─ 矿工发送 V2 心跳，更新 last_heartbeat
  └─ bucket 逐块增长

HEARTBEAT_V2_HEIGHT + MINER_EPOCH_LENGTH (第二个 V2 epoch 边界):
  ├─ 首个正常结算
  ├─ 签到判断: last_heartbeat / EPOCH_LENGTH == settled_epoch
  ├─ 签到 → attendance |= 1, misses = 0
  ├─ 缺席 → reputation 衰减, misses++
  ├─ bucket → 分配到 pending
  └─ 正常运行

所有节点必须在 V2_HEIGHT 前升级（version-manifest critical_minimum 强制）
```

**注意**：这要求 `HEARTBEAT_V2_HEIGHT % MINER_EPOCH_LENGTH == 0`，而之前要求不整除。
新设计中整除是**必须的**——因为激活和 epoch 边界必须是同一个块。

---

## 时间线验证

### 场景 1: 旧 snapshot 加载（V2_HEIGHT 前）

```
高度 H < V2_HEIGHT:
  节点加载旧 snapshot → V1→V2 迁移 → MinerInfo 有 V2 字段（任意值）
  state_root(H) 使用 borsh_v1() → 只序列化 V1 字段 → hash 与旧区块一致 ✓
```

### 场景 2: fork 时刻（不同升级时间的节点）

```
节点 A（早升级）: 新矿工 V2 字段 = 零（handle_miner_register 写入）
节点 B（晚升级）: 迁移矿工 V2 字段 = 0xFFFF（From<MinerInfoV1> 写入）

V2_HEIGHT 前: state_root 用 borsh_v1() → V2 字段不参与 → 一致 ✓
V2_HEIGHT 到达 (同时是 epoch 边界):
  归一化运行 → 所有矿工 V2 字段统一设为零 → 覆盖 0xFFFF 和 0
  state_root 切换到 V2 → 所有节点序列化相同 → 一致 ✓
  
没有"V2_HEIGHT 到 epoch 边界"的不一致窗口——因为 V2_HEIGHT 本身就是 epoch 边界。
```

### 场景 3: mid-epoch 重启（V2_HEIGHT 后）

```
节点在 epoch 中间重启:
  加载 snapshot:
    ├─ MinerInfo V2 字段（含 last_heartbeat）→ 从 snapshot 恢复 ✓
    ├─ epoch_reward_bucket → 从 snapshot 恢复（已持久化）✓
    ├─ epoch_checkins → 丢失（borsh skip）→ 仅影响去重缓存，不影响共识
    └─ state_root 验证: 包含 MinerInfo V2 + epoch_reward_bucket → 一致 ✓
  
  下一个 epoch 边界:
    ├─ 签到判断用 last_heartbeat 落在被结算 epoch 范围内 → 确定性一致 ✓
    ├─ epoch_reward_bucket → snapshot 值 + 重启后新 per-block 累加 → 正确 ✓
    └─ 与未重启节点计算完全相同 → 无共识分裂 ✓

epoch_checkins 丢失的影响:
  - 矿工在重启前的心跳已写入 last_heartbeat（持久化）→ 签到记录不丢失
  - epoch_checkins 只用于 handler 内去重（防同 epoch 重复心跳）
  - 重启后矿工可能在同 epoch 再发一次心跳 → last_heartbeat 更新但无害
  - 完全重启安全 ✓
```

### 场景 4: 已部署 v0.4.21 的中间 snapshot 格式（第五轮修订 #3）

```
v0.4.21 短暂运行期间可能写了 snapshot:
  MinerInfo = V2 (12 字段), epoch_reward_bucket 不在 borsh 中 (skip)

这是第三种格式，需要处理:
  V1: MinerInfo 8 字段, 无 bucket → 旧节点
  V2-interim: MinerInfo 12 字段, 无 bucket → v0.4.21 短暂写入
  V2-final: MinerInfo 12 字段, 有 bucket → 最终版本

V2-interim snapshot 不可安全恢复：
  - epoch_reward_bucket 从未序列化，无法从 snapshot 中恢复
  - 加载时 bucket=0 但 pool 已被扣款 → 供应量不守恒
  - 即使回退 bucket=0，state_root 也无法匹配（bucket 参与 state_root）

处理方式：**V2-interim 格式不支持**。
  - from_snapshot_bytes 只支持两种格式: V2-final 和 V1
  - 遇到 V2-interim → 反序列化失败 → 节点报错要求从 v0.4.20 备份恢复
  - 主网/testnet 已全部回退到 v0.4.20 备份，不存在 V2-interim snapshot
  - 部署文档明确要求：升级前必须备份，不支持从 v0.4.21 snapshot 恢复
```

---

## 可扩展性

未来任何结构体需要新增字段时，遵循相同模式：

1. 给结构体加新字段
2. 添加 `borsh_vN()` 方法序列化旧版字段
3. 在 `state_root()` 中用 `if height >= FORK_HEIGHT` 切换序列化
4. 在第一个 fork epoch boundary 运行归一化（overwrite 所有新字段为确定性值）
5. state_root V2 切换延迟到归一化完成的块
6. 在 `from_snapshot_bytes()` 中加 VN→VN+1 迁移
7. version-manifest critical_minimum 强制升级

关键约束：
- **影响共识的字段必须持久化**（不能 borsh(skip) 后又用于边界决策）
- **共识决策只依赖持久化数据**（用 last_heartbeat 而非 epoch_checkins）
- **瞬态字段（borsh skip）只能做运行时缓存**（如去重、性能优化）
- **state_root 序列化切换必须和归一化同时发生**（避免不一致窗口）
- **归一化必须是确定性的**（不依赖迁移时的值，用固定常量）
- **不改变货币政策**（per-block 份额保留，不改为边界一次性结算）

---

## 需要修改的文件

| 文件 | 改动 |
|------|------|
| `types/state.rs` | MinerInfo 保留 V2 字段 + `borsh_v1()` 方法；回退 MinerEpochState 拆分 |
| `state/world.rs` | `epoch_reward_bucket` 去掉 `#[borsh(skip)]` 持久化；`state_root()` 按 height 切换 MinerInfo + bucket 的序列化；V1→V2 WorldState 迁移 |
| `state/rewards.rs` | `process_miner_epoch_boundary`: 签到判断用 `last_heartbeat / EPOCH_LENGTH == settled_epoch`；激活块特殊处理（归一化+归零bucket+跳过结算）；保留 per-block `accumulate_mining_reward` |
| `state/handlers.rs` | V2 心跳准入改为 epoch-number 判断（`last_epoch < current_epoch`），消除相位锁定 |
| `node/chain.rs` | 保持 epoch boundary 在 state_root 之前；保持 per-block accumulate 调用 |
| `state/tests.rs` | 更新测试；新增 mid-epoch 重启场景测试 |

---

## 测试策略（testnet 先行）

之前的教训：方案未经充分测试就部署主网导致事故。

### 四级验证

1. **单元测试** (cargo test)
   - V1→V2 snapshot 迁移 round-trip
   - state_root V1/V2 序列化切换
   - epoch boundary off-by-one 测试（边界块心跳、早签到、晚签到）
   - mid-epoch restart 模拟（构造 snapshot → 加载 → 验证 state_root）
   - 归一化确定性测试（不同迁移路径 → 归一化后相同）
   - supply integrity（bucket + pending 不破坏总供应量）

2. **Testnet 部署验证**
   - 部署到 Hetzner testnet 节点
   - 设 HEARTBEAT_V2_HEIGHT = `round_up(testnet_height + 200, MINER_EPOCH_LENGTH)` 确保对齐 epoch 边界
   - 代码中加启动断言: `assert!(HEARTBEAT_V2_HEIGHT % MINER_EPOCH_LENGTH == 0)`
   - 观察：
     - [x] 旧 snapshot 加载成功（V1→V2 迁移）
     - [x] V2_HEIGHT 前 state_root 一致（borsh_v1）
     - [x] V2_HEIGHT 到达时归一化运行
     - [x] 第一个 V2 epoch 正常结算
     - [x] 重启节点 → state_root 验证通过
     - [x] 矿工心跳 + pending 确认/作废
   - 至少运行 1 小时（12 个 epoch）确认稳定

3. **Testnet 重启测试**
   - 在 V2_HEIGHT 后、epoch 中间手动重启节点
   - 验证 state_root 不 mismatch
   - 验证 epoch_reward_bucket 恢复正确
   - 验证下一个 epoch 边界结算正确

4. **Mainnet 部署**
   - testnet 验证全部通过后
   - 备份所有节点 data/ 目录
   - 部署 + 监控

---

## 回滚方案

如果升级后出问题：
1. 停止所有节点
2. 恢复 chain.redb 备份（升级前必须备份）
3. 回退到旧版本二进制
4. 旧版本加载旧 snapshot → state_root 使用 V1 序列化 → 一致 ✓
5. 新版本 snapshot 中 MinerInfo 多了字段，旧版本无法读取 → 必须用备份

---

## Codex 审核修订历史

### 第三轮修订

| # | 原方案问题 | 修订内容 |
|---|-----------|---------|
| 1 | epoch_checkins/epoch_reward_bucket 是 borsh(skip) 但参与 state_root | 瞬态字段不参与 state_root；边界一次性结算 |
| 2 | 早升级 vs 晚升级节点 V2 字段不同 → fork 共识分裂 | 第一个 V2 epoch 边界强制归一化 |
| 3 | MinerEpochState 拆分不完整 | 回退拆分（代码问题） |

### 第四轮修订

| # | 第三轮方案的新问题 | 修订内容 |
|---|-------------------|---------|
| 1 | epoch_checkins 丢失 → 边界决策不同 → **共识分裂** | **边界决策改用 `last_heartbeat >= epoch_start`**（已持久化），epoch_checkins 降级为运行时去重缓存 |
| 2 | 边界一次性结算 → validators 先消耗 pool → **矿工收益被剥夺** | **恢复 per-block 扣除**（accumulate_mining_reward），`epoch_reward_bucket` 持久化（去掉 borsh skip） |
| 3 | 代码中 MinerEpochState 拆分不完整 | 回退拆分，保留 V2 字段在 MinerInfo 中 |

### 第五轮修订

| # | 第四轮方案的新问题 | 修订内容 |
|---|-------------------|---------|
| 1 | MinerEpochState 拆分不完整（代码） | 回退，保留 V2 字段在 MinerInfo |
| 2 | `last_heartbeat >= epoch_start` off-by-one：边界块属于新 epoch | **改为检查 `last_heartbeat` 落在被结算 epoch 范围内**（`settled_epoch = current - 1`） |
| 3 | v0.4.21 写了第三种 snapshot 格式（V2 MinerInfo 无 bucket） | **from_snapshot_bytes 三级回退**：V2-final → V2-interim → V1 |
| 4 | V2_HEIGHT 到第一个 epoch 边界有未提交状态的窗口 | **统一激活：V2_HEIGHT 本身必须是 epoch 边界**，所有 V2 逻辑原子激活 |

### 第六轮修订

| # | 第五轮方案的新问题 | 修订内容 |
|---|-------------------|---------|
| 1 | MinerInfo 拆分不完整（代码） | 已知，开始写代码时第一步回退 |
| 2 | fork 块 accumulate 在 epoch 结算前执行 → 1 块奖励被立即结算 | **激活块归零 epoch_reward_bucket**（退回 pool） |
| 3 | V1 矿工 last_heartbeat 间隔 1000 块，V2 用 100 块窗口判断 → 误判缺席 | **激活块跳过签到/缺席/衰减**，只做归一化+初始化 |

### 第七轮修订

| # | 第六轮方案的新问题 | 修订内容 |
|---|-------------------|---------|
| 1 | MinerInfo 拆分不完整（代码） | 已知，开始写代码时第一步回退 |
| 2 | epoch_checkins/bucket 仍在 state_root 中且 borsh(skip)（代码） | 已知，代码未按新方案修改 |
| 3 | V2 心跳 `last_heartbeat + 100` 相位锁定 → 漂移导致误判缺席 | **改为 epoch-number 判断**：`last_epoch < current_epoch` 即允许签到 |

### 设计原则（从七轮审核中提炼）

1. **影响共识的数据必须持久化** — 不能 borsh(skip) 后又用于边界决策
2. **共识决策只依赖持久化数据** — 用 last_heartbeat 而非 epoch_checkins
3. **borsh(skip) 字段只能做运行时缓存** — 丢失不能影响正确性
4. **不改变货币政策** — 技术重构不应改变经济模型
5. **所有 V2 逻辑原子激活** — 不留"部分激活"窗口
6. **归一化用固定常量** — 不依赖迁移路径的值
7. **testnet 先行** — 任何 fork 升级必须先在 testnet 完整验证
8. **边界检查必须明确"哪个 epoch"** — 边界块属于新 epoch，结算的是上一个
9. **激活块只初始化不结算** — 避免用旧规则数据做新规则判断
10. **激活块清理 in-flight 状态** — 归零 bucket 等中间状态
11. **准入逻辑用 epoch-number 而非 delta** — 避免相位锁定和漂移
