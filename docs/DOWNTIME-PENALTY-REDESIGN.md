# Downtime 惩罚机制重设计

> 日期: 2026-03-22
> 状态: 已确认
> 不需要重置: 仅行为变更，无 state schema 变化

## 背景

v0.3.0 的 downtime 惩罚过于严厉：miss 50% → slash 1% + jail。
导致跨国节点因网络延迟被反复 slash，每个 epoch 损失 1% 质押。

## 业界对比

| 公链 | 离线惩罚 | 阈值 | 窗口 | 罚金 |
|------|---------|------|------|------|
| Cosmos | slash | miss 95% | 10,000 blocks 滑动窗口 | 0.01% |
| Ethereum | 不 slash | — | inactivity leak 仅在无法 finalize 时 | 极小 |
| Polkadot | 不 slash | — | — | 只扣奖励 |
| Solana | 不 slash | — | — | 只扣奖励 |
| **我们 v0.3.0** | **slash** | **miss 50%** | **100 blocks** | **1%** |

## 新方案

### 核心原则

**离线 ≠ 恶意。Slash 只用于双签。离线只扣奖励。**

### 变更内容

| 行为 | 旧 (v0.3.0) | 新 (v0.3.1) |
|------|------------|------------|
| 离线/丢块 | slash 1% + jail 100 blocks | 不发 block reward，不 slash，不 jail |
| 双签 (equivocation) | slash 10% + jail | 不变 |
| 长期离线 | 无 | 移出 active set（不 slash） |

### 具体实现

#### 1. `slashing.rs`: `process_downtime_slashing` → `process_downtime_penalties`

不再调用 `slash_stake`，不再 jail。只返回被判定离线的 validator 列表，
供奖励分发时排除。

```rust
// 之前: slash + jail
let slashed = slash_stake(stakes, validator, DOWNTIME_SLASH_BPS);
self.jailed.insert(*validator, current_height + JAIL_DURATION);

// 之后: 只标记为离线，不 slash，不 jail
offline_validators.push(*validator);
tracing::info!("Validator offline — excluded from rewards");
```

#### 2. `rewards.rs`: `distribute_block_reward` 排除离线 validator

奖励分发时，跳过被标记为离线的 validator。它们的份额归还 pool（不分给别人）。

#### 3. `chain.rs`: 传递 offline 列表给奖励分发

在 epoch 边界，先计算 offline validators，再把列表传给下个 epoch 的奖励分发。

### 不变的部分

- `EquivocationEvidence` + `report_equivocation` → 10% slash + jail，不变
- `SlashingState` 结构体，不变
- WorldState schema，不变（无需重置）
- `assigned_slots` / `missed_slots` 追踪，不变（仍然需要判断谁离线）

### 长期路线

当网络成长到 20+ 外部 validator 后，可考虑加回温和的 Cosmos 式 downtime slash:
- 0.01% 罚金（当前的 1/100）
- 95% miss 阈值
- 滑动窗口 10,000 blocks
