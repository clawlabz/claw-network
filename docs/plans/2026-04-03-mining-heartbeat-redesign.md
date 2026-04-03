# Mining Heartbeat Redesign: Epoch 计分 + 延迟结算 + 声誉衰减

**日期**: 2026-04-03
**状态**: 待确认 (Codex 审核 4 问题已修复)
**动机**: 当前心跳机制存在白嫖漏洞——矿工发完心跳后关机，可免费领取最多 100 分钟挖矿奖励

---

## 问题分析

### 当前机制

```
矿工发心跳 → active=true → 持续拿奖励 → 2000块(~100分钟)后才检查
```

| 参数 | 当前值 | 真实时间 |
|------|--------|---------|
| `MINER_HEARTBEAT_INTERVAL` | 1,000 块 | ~50 分钟 |
| `MINER_GRACE_BLOCKS` | 2,000 块 | ~100 分钟 |
| 客户端发送间隔 | 3,000 秒 | 50 分钟 |

### 漏洞

- 矿工发完心跳立刻关机 → 链上要等 2000 块才标记 inactive → 白嫖 100 分钟奖励
- 最坏情况：每 100 分钟开机发一次心跳就能持续白嫖
- 对诚实矿工不公平，稀释了他们的奖励份额

### 行业对标

| 链 | 机制 | 检测速度 | 奖励延迟 |
|---|---|---|---|
| Ethereum 2.0 | 每 epoch attestation，缺席扣钱 | 秒级 | 共识层累积 |
| Cosmos | 10,000 块签名窗口，<5% jail | 分钟级 | distribution module 手动领取 |
| Solana | Vote Credits，及时性加权 | 秒级 | 2-3 天/epoch |
| Filecoin | 每 30 分钟提交存储证明 | 30 分钟 | 75% 锁仓 180 天释放 |
| Polkadot | 按 Era 结算，需主动领取 | Era 级 | 6 小时/era |
| Helium | 随机 challenge 无线电响应 | 72 小时 | beacon 后结算 |

**行业共识**: 没有一条链是"发完心跳就持续给钱"的。都是**持续证明参与 → 才给奖励**。奖励延迟发放是标准做法。

---

## 新方案设计

### 核心思想转变

```
当前:  心跳 → 开关 (active=true/false) → 持续发钱
新方案: 心跳 → 累积积分 → 按积分结算 → 没积分就没钱
```

### 1. Epoch 划分

```
EPOCH_LENGTH = 100 块 (~5 分钟)
每个 epoch 是一个独立的奖励结算单元
```

### 2. 心跳 → Epoch 签到

```
MINER_HEARTBEAT_INTERVAL: 1000 → 100 块 (每 epoch 可签到一次)
客户端发送间隔: 50 分钟 → 4 分钟
```

矿工每个 epoch 必须签到一次。没签到的 epoch = 没有这个 epoch 的奖励资格。

### 3. 奖励延迟结算 (Pending → Confirmed)

杜绝白嫖的关键机制:

```
Epoch N:   矿工发心跳 → 记录签到
Epoch N 结束: 计算该 epoch 奖励 → 进入 pending_rewards
Epoch N+1: 矿工再次发心跳 → 确认 Epoch N 的 pending → 转入余额
           如果 Epoch N+1 没心跳 → Epoch N 的 pending 作废回国库
```

效果: 矿工必须**连续两个 epoch 都在线**才能拿到第一个 epoch 的奖励。发完心跳立刻关机 → pending 永远无法确认 → 一分钱拿不到。

### 4. 滑动窗口声誉 (Uptime Score)

```
UPTIME_WINDOW = 12 个 epoch (~1 小时)
uptime_score = 过去 12 个 epoch 中成功签到的次数 / 12
```

uptime_score 影响奖励倍率:

| uptime_score | 奖励倍率 | 含义 |
|---|---|---|
| 100% (12/12) | 1.0x | 全勤 |
| ≥75% (9-11/12) | 0.8x | 偶尔断线 |
| ≥50% (6-8/12) | 0.5x | 不稳定 |
| <50% | 0x | 不参与分奖励 |

效果: 频繁上线/下线刷心跳的矿工 uptime_score 低，拿不到多少奖励。鼓励长期稳定在线。

### 5. 声誉衰减 (Reputation Decay)

当前 reputation 只升不降，改为双向:

```
每个 epoch 结算时:
  签到 → reputation_bps 按现有规则升级 (newcomer → established → veteran)
  缺席 → reputation_bps 衰减 1% (向 NEWCOMER 方向衰减)
  连续缺席 > 6 个 epoch (~30分钟) → active = false + 信誉降一档
```

效果: veteran 矿工也不能躺着吃老本，持续缺席会掉回 established 甚至 newcomer。

---

## 数据结构变化

### MinerInfo 结构 (crates/types/src/state.rs)

```rust
pub struct MinerInfo {
    // --- 现有字段保留 ---
    pub address: [u8; 32],
    pub tier: MinerTier,
    pub name: String,
    pub registered_at: u64,
    pub last_heartbeat: u64,
    pub ip_prefix: Vec<u8>,
    pub active: bool,
    pub reputation_bps: u16,

    // --- 新增字段 ---
    /// 待确认奖励 (需下一个 epoch 心跳确认)
    pub pending_rewards: u128,
    /// pending_rewards 对应的 epoch 编号
    pub pending_epoch: u64,
    /// 过去 16 个 epoch 的签到位图 (最低位 = 最近 epoch)
    pub epoch_attendance: u16,
    /// 连续缺席 epoch 计数
    pub consecutive_misses: u16,
}
```

### 常量变化 (crates/types/src/state.rs)

```rust
// --- 修改 ---
pub const MINER_HEARTBEAT_INTERVAL: u64 = 100;   // 原 1,000 → 每 epoch 一次
pub const MINER_GRACE_BLOCKS: u64 = 600;          // 原 2,000 → 6 个 epoch

// --- 新增 ---
pub const EPOCH_LENGTH: u64 = 100;                // ~5 分钟
pub const UPTIME_WINDOW: u32 = 12;                // 看过去 12 个 epoch (~1小时)
pub const MIN_UPTIME_FOR_REWARD: u32 = 6;         // 至少 50% 出勤率
pub const MINER_GRACE_EPOCHS: u16 = 6;            // 连续缺席 6 epoch → 下线
pub const REPUTATION_DECAY_BPS: u16 = 100;        // 缺席一次衰减 1%

// --- 保留不变 ---
pub const REPUTATION_NEWCOMER_BPS: u16 = 2_000;
pub const REPUTATION_ESTABLISHED_BPS: u16 = 5_000;
pub const REPUTATION_VETERAN_BPS: u16 = 10_000;
pub const MAX_MINERS_PER_SUBNET: usize = 3;
```

### WorldState 新增 (crates/state/src/world.rs)

```rust
/// 当前 epoch 的矿工签到记录: address → true
pub epoch_checkins: BTreeMap<[u8; 32], bool>,

/// 当前 epoch 累加的挖矿奖励 (每块累加，epoch 边界分配)
pub epoch_reward_bucket: u128,
```

---

## 奖励流程

### 逐块累加 (每个块执行，替代原 distribute_mining_rewards)

当前代码每块调用 `distribute_mining_rewards()` 直接发钱给 active 矿工。
新设计改为：**每块累加到 epoch bucket，不直接发给矿工**。

```
fn accumulate_mining_reward(world: &mut WorldState, height: u64):
  if height < HEARTBEAT_V2_HEIGHT:
    return distribute_mining_rewards(world, height)  // 旧逻辑不变

  base_reward = reward_per_block(height)
  mining_reward = base_reward * MINING_REWARD_BPS / 10000
  actual = min(mining_reward, pool_balance)
  if actual == 0: return

  // 从池子扣除，累加到 epoch bucket
  balances[pool] -= actual
  world.epoch_reward_bucket += actual
```

这样自然处理了：
- **减半边界**: 每块用自己高度的 `reward_per_block()`，跨减半的 epoch 自动正确
- **池子耗尽**: 每块 `min(mining_reward, pool_balance)`，池子不够自动停
- **精度**: 与原逻辑完全一致，无舍入差异

### Epoch 边界结算 (每 EPOCH_LENGTH 块触发一次)

**执行时机**: 必须在 `state_root()` 计算之前（见下方"执行顺序"章节）。

```
fn process_epoch_boundary(world: &mut WorldState, height: u64):
  if height < HEARTBEAT_V2_HEIGHT: return
  current_epoch = height / EPOCH_LENGTH
  if height % EPOCH_LENGTH != 0: return

  for each (addr, miner) in world.miners:
    checked_in = epoch_checkins.contains(addr)

    if checked_in:
      // 1. 确认上期 pending → 转余额
      if miner.pending_epoch == current_epoch - 1 && miner.pending_rewards > 0:
        balances[addr] += miner.pending_rewards
        emit RewardDistributed(addr, miner.pending_rewards, "mining_reward_confirmed")

      // 2. 重新激活 + 重置计数
      miner.active = true    // ← 关键: 恢复因缺席被停用的矿工
      miner.consecutive_misses = 0
      miner.epoch_attendance = (miner.epoch_attendance << 1) | 1

    else:
      // 1. 上期 pending 作废 → 回国库
      if miner.pending_rewards > 0:
        balances[pool] += miner.pending_rewards
        emit RewardForfeited(addr, miner.pending_rewards)

      // 2. 缺席处理
      miner.consecutive_misses += 1
      miner.epoch_attendance = miner.epoch_attendance << 1  // 最低位 0
      miner.reputation_bps = max(NEWCOMER, miner.reputation_bps * 99 / 100)

      // 3. 连续缺席过多 → 下线
      if miner.consecutive_misses >= MINER_GRACE_EPOCHS:
        miner.active = false

    // 重置 pending
    miner.pending_rewards = 0
    miner.pending_epoch = current_epoch

  // 计算 uptime 并分配 epoch bucket 到各矿工的 pending_rewards
  uptime_qualified_miners = []
  for (addr, miner) in world.miners where miner.active:
    attendance_bits = miner.epoch_attendance & 0x0FFF  // 低 12 位
    uptime_count = popcount(attendance_bits)
    if uptime_count >= MIN_UPTIME_FOR_REWARD:
      uptime_multiplier = uptime_tier(uptime_count)
      weight = tier_weight * reputation_bps * uptime_multiplier
      uptime_qualified_miners.push((addr, weight))

  // 按权重将 epoch_reward_bucket 分配到各矿工的 pending_rewards
  total_weight = sum(weights)
  for (addr, weight) in uptime_qualified_miners:
    share = epoch_reward_bucket * weight / total_weight
    miners[addr].pending_rewards = share

  // 未分配的余额（无合格矿工时）回国库
  if uptime_qualified_miners.is_empty():
    balances[pool] += epoch_reward_bucket

  // 清空
  world.epoch_reward_bucket = 0
  world.epoch_checkins.clear()
```

### Uptime 倍率函数

```rust
fn uptime_tier(count: u32) -> u128 {
    match count {
        12 => 100,      // 1.0x
        9..=11 => 80,   // 0.8x
        6..=8 => 50,    // 0.5x
        _ => 0,         // 不参与
    }
}
```

---

---

## 执行顺序 (Codex 审核 CRITICAL #2 修复)

当前代码的 epoch 处理（`update_miner_activity`）在 `state_root()` 和 `put_block_and_snapshot()` **之后**执行，
导致 epoch 变更不在已提交的 state_root 内。对于仅改 active 标志的旧逻辑影响有限，
但新设计涉及余额变更（pending→余额、作废→国库），必须修正执行顺序。

### produce_block 中的新执行顺序

```
1. 应用交易 (apply pending transactions)
2. 分配验证者奖励 (distribute_block_reward, distribute_fees)
3. 累加挖矿奖励到 epoch bucket (accumulate_mining_reward)  ← 替代原 distribute_mining_rewards
4. ★ Epoch 边界结算 (process_epoch_boundary)               ← 移到 state_root 之前
5. ★ Miner activity 更新 (update_miner_activity)           ← 移到 state_root 之前
6. sync_slashing_to_world_state
7. state_root = inner.state.state_root()                    ← 包含 epoch 结算结果
8. 创建 Block + 签名
9. supply integrity check
10. put_block_and_snapshot (原子持久化)
11. validator set rotation (不涉及 state_root 的操作可以留在后面)
```

### apply_remote_block 中的新执行顺序

```
1. 应用交易到 state_clone
2. 分配验证者奖励
3. 累加挖矿奖励到 epoch bucket
4. ★ Epoch 边界结算 (process_epoch_boundary)
5. ★ Miner activity 更新 (update_miner_activity)
6. sync_slashing_to_world_state
7. computed_root = state_clone.state_root()
8. 验证 computed_root == block.state_root                   ← 两端一致
9. supply integrity check
10. accept block + persist
11. validator set rotation
```

### 关键约束

- `process_epoch_boundary` 和 `update_miner_activity` **必须**在 `state_root()` 之前
- 两条路径（produce + apply）的执行顺序**必须完全一致**
- epoch 结算产生的 events 放入 `block.events`
- 崩溃恢复安全：所有余额变更都在 state_root 内，snapshot 是一致的

---

## 状态迁移策略 (Codex 审核 CRITICAL #1 修复)

### 问题

`MinerInfo` 使用 `#[derive(BorshSerialize, BorshDeserialize)]`，Borsh 是位置序列化。
新增 4 个字段会改变二进制布局，升级后的节点无法反序列化旧 snapshot。

### 方案: 版本化 MinerInfo + 一次性迁移

```rust
/// 旧版结构 (用于反序列化已有 snapshot)
#[derive(BorshDeserialize)]
pub struct MinerInfoV1 {
    pub address: [u8; 32],
    pub tier: MinerTier,
    pub name: String,
    pub registered_at: u64,
    pub last_heartbeat: u64,
    pub ip_prefix: Vec<u8>,
    pub active: bool,
    pub reputation_bps: u16,
}

/// 新版结构
#[derive(BorshSerialize, BorshDeserialize)]
pub struct MinerInfo {
    // V1 字段
    pub address: [u8; 32],
    pub tier: MinerTier,
    pub name: String,
    pub registered_at: u64,
    pub last_heartbeat: u64,
    pub ip_prefix: Vec<u8>,
    pub active: bool,
    pub reputation_bps: u16,
    // V2 新增
    pub pending_rewards: u128,
    pub pending_epoch: u64,
    pub epoch_attendance: u16,
    pub consecutive_misses: u16,
}

impl From<MinerInfoV1> for MinerInfo {
    fn from(v1: MinerInfoV1) -> Self {
        Self {
            // 复制 V1 字段...
            // V2 字段初始化:
            pending_rewards: 0,
            pending_epoch: 0,
            epoch_attendance: if v1.active { 0xFFFF } else { 0 },  // 活跃矿工视为全勤
            consecutive_misses: if v1.active { 0 } else { 6 },
        }
    }
}
```

### WorldState 迁移

同理，`WorldState` 新增 `epoch_checkins` 和 `epoch_reward_bucket` 字段。

```rust
/// WorldState 反序列化时尝试新格式，失败则回退旧格式 + 迁移
pub fn load_state(bytes: &[u8]) -> Result<WorldState> {
    // 先尝试 V2
    match WorldState::try_from_slice(bytes) {
        Ok(state) => Ok(state),
        Err(_) => {
            // 回退 V1 → 迁移
            let v1 = WorldStateV1::try_from_slice(bytes)?;
            Ok(WorldState::migrate_from_v1(v1))
        }
    }
}
```

### 迁移时机

- 节点启动加载 snapshot 时自动检测版本
- 迁移后的第一个 snapshot 写入为新格式
- 无需额外的迁移命令或停机
- 一旦全网升级完成，V1 反序列化代码可在后续版本移除

### 回滚方案

- 如果升级出问题，节点可回退到旧版本二进制
- 旧版本加载旧 snapshot（升级前的最后一个），丢弃升级后的块
- 需要从其他旧版本节点同步追赶
- **建议**: 升级前备份各节点的 `data/` 目录

---

## 白嫖场景验证

| 攻击方式 | 当前结果 | 新方案结果 |
|---|---|---|
| 发心跳后立刻关机 | 白嫖 100 分钟 | pending 作废，0 收益 |
| 每 50 分钟上线刷一次 | 持续拿满额奖励 | uptime 8%，低于 50%，0 收益 |
| 每 10 分钟上线刷一次 | 持续拿满额奖励 | uptime ~17%，低于 50%，0 收益 |
| 每 5 分钟上线刷一次 | 持续拿满额奖励 | uptime 100%，但成本与持续在线相同 |
| 稳定在线偶尔断 5 分钟 | 与全勤相同 | uptime 92%，拿 0.8x，合理 |

---

## 客户端改动

### claw-miner (Python)

```python
# constants.py
HEARTBEAT_INTERVAL_SECONDS: int = 4 * 60  # 50分钟 → 4分钟
```

### clawnetwork-openclaw (TypeScript)

```typescript
const MINER_HEARTBEAT_INTERVAL_MS = 4 * 60 * 1000  // 50分钟 → 4分钟
```

---

## 迁移策略

### 升级高度 (Hard Fork)

需要设定一个 `HEARTBEAT_V2_HEIGHT`，在该高度后切换到新逻辑:

```rust
pub const HEARTBEAT_V2_HEIGHT: u64 = ???;  // 待定，需协调所有节点升级
```

### 行为分叉点

| 组件 | `< HEARTBEAT_V2_HEIGHT` | `>= HEARTBEAT_V2_HEIGHT` |
|------|-------------------------|--------------------------|
| `accumulate_mining_reward` | 调用旧 `distribute_mining_rewards` (逐块直发) | 累加到 `epoch_reward_bucket` |
| `process_epoch_boundary` | 不执行 | 执行 epoch 结算 |
| `handle_miner_heartbeat` | 旧间隔校验 (1000块) | 新间隔校验 (100块) + 记录 epoch_checkins |
| `update_miner_activity` | 旧 grace period (2000块) | 由 epoch 结算中的 consecutive_misses 替代 |

### 状态迁移

详见上方"状态迁移策略"章节。节点启动时自动检测 snapshot 版本并迁移，无需停机。

### 客户端兼容

- 客户端需**提前**升级到新心跳间隔 (4分钟)
- 新间隔客户端在旧链上运行：每 4 分钟发心跳，但旧链 1000 块间隔限制下只有每 ~50 分钟的那次能成功，中间的会收到 `HeartbeatTooEarly` 错误，**不影响功能**
- 旧间隔客户端在新链上运行：每 50 分钟发一次心跳，只能签到 1/10 的 epoch，uptime 10% < 50% 阈值，**无法获得奖励**。这就是升级的激励

### 升级顺序

1. 发布新版 claw-miner + clawnetwork-openclaw (新心跳间隔)
2. 通知矿工升级客户端（公告明确：不升级 = 无奖励）
3. 备份所有验证者节点的 `data/` 目录
4. 升级所有 6 个验证者节点 (新 claw-node 二进制)
5. 到达 HEARTBEAT_V2_HEIGHT → 新逻辑自动激活
6. 监控 1 小时确认全网状态一致

---

## 需要修改的文件清单

### 核心代码 (claw-node Rust)

| 文件 | 改动 |
|------|------|
| `crates/types/src/state.rs` | MinerInfo 新增字段 + 新常量 + 修改旧常量 |
| `crates/types/src/transaction.rs` | MinerHeartbeatPayload 可能不变 |
| `crates/state/src/handlers.rs` | handle_miner_heartbeat 适配新逻辑 |
| `crates/state/src/rewards.rs` | distribute_mining_rewards → accumulate + epoch 结算 + pending 机制 |
| `crates/state/src/world.rs` | WorldState 新增 epoch_checkins / epoch_reward_bucket + V1 迁移 |
| `crates/state/src/tests.rs` | 大量测试更新/新增 |
| `crates/node/src/chain.rs` | **执行顺序重排**: epoch 结算移到 state_root 之前 (produce + apply 两端) |

### 客户端代码

| 文件 | 改动 |
|------|------|
| `claw-miner/src/clawminer/constants.py` | HEARTBEAT_INTERVAL_SECONDS: 3000 → 240 |
| `clawnetwork-openclaw/index.ts` | MINER_HEARTBEAT_INTERVAL_MS: 50min → 4min |

### 文档

| 文件 | 改动 |
|------|------|
| `claw-miner/README.md` | 心跳间隔描述 50分钟 → 4分钟 |
| `claw-miner/docs/install.md` | 安装文档心跳间隔更新 |
| `clawnetwork-openclaw/README.md` | 插件行为描述更新 |
| `docs/PROTOCOL.md` | 补充 MinerHeartbeat 协议文档 (当前缺失) |
| `docs/VALIDATOR-ECONOMICS.md` | 矿工奖励机制更新 |
| `docs/plans/2026-03-22-agent-mining-design.md` | 引用本文档为新版设计 |

### 内存/运维

| 文件 | 改动 |
|------|------|
| `memory/project_openclaw_heartbeat_bug.md` | 更新挖矿经济机制分析部分 |
| `memory/project_clawnetwork_economics_plan.md` | 更新心跳/奖励参数 |
