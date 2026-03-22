# ClawNetwork 质押模块重构计划

> 审查日期: 2026-03-22
> 状态: **已确认，执行中**
> 范围: 6 CRITICAL/HIGH 问题 + 4 MEDIUM 问题一次性解决
> 预计: 主网重置（无真实用户，无兼容负担）

---

## 核心问题

当前质押相关状态分散在三个地方，互相不同步：

```
WorldState.stakes          — 链上持久化的质押金额
ValidatorSet.candidates    — 内存中的验证者候选人
SlashingState              — 内存中的惩罚状态（不持久化）
unbonding_queue            — 解绑中的资金（不计入 total_supply）
```

**根因：应该有一个 Single Source of Truth，而不是三套账本互相 sync。**

---

## 重构方案

### 原则
- WorldState 是唯一的质押状态源
- ValidatorSet 只是 WorldState.stakes 的视图/缓存，不独立维护 candidates
- SlashingState 持久化到 WorldState
- total_supply() = balances + stakes + unbonding

### Phase 0: ChangeDelegation 安全加固（解决 #3）— 独立修复，即时价值

**问题**: 验证者可以调 ChangeDelegation 偷走委托人的资金。

**修复**: ChangeDelegation 只允许当前委托人发起，验证者不能单方面修改：

```rust
// 之前: 允许 delegator OR validator
if tx.from != current_delegator && tx.from != payload.validator {
    return Err("not authorized");
}

// 之后: 只允许 delegator（或 self-stake 的 validator 本人）
let is_self_stake = current_delegator == payload.validator;
if tx.from != current_delegator && !(is_self_stake && tx.from == payload.validator) {
    return Err("not authorized: only the current delegator can change delegation");
}
```

逻辑：
- 外部委托（Owner delegate 给 Validator）: 只有 Owner 能改
- 自委托（Validator self-stake）: Validator 自己能改（允许转为外部委托）

**注意**: `stake_delegations` 总是在 `handle_stake_deposit` 时通过 `or_insert(tx.from)` 写入，
所以正常流程不会出现缺失记录的情况。主网重置后无需考虑历史遗留数据。

### Phase 1: 统一质押状态（解决 #1 #2 #4）

**1.1 删除 ValidatorSet.candidates**

ValidatorSet 不再维护自己的 candidates BTreeMap。`recalculate_active` 直接读 WorldState.stakes：

```rust
// 之前: ValidatorSet 自己维护 candidates
pub struct ValidatorSet {
    candidates: BTreeMap<[u8; 32], StakeInfo>,  // 删除
    active: Vec<ActiveValidator>,
    epoch: u64,
    weight_config: WeightConfig,
}

// 之后: recalculate_active 接收 &WorldState.stakes 作为参数
pub fn recalculate_active(
    &mut self,
    stakes: &BTreeMap<[u8; 32], u128>,
    slashing: &SlashingState,
    reputation: &[ReputationAttestation],
    current_height: u64,
) { ... }
```

**影响**:
- 删除 `sync_validator_stakes()` — 不再需要
- 删除 `ValidatorSet::stake()` / `unstake()` / `slash()` — 直接操作 WorldState
- `with_initial_stakes` 改为接收 &stakes 引用
- **`compute_weight()` 签名同步修改**: 从 `&BTreeMap<[u8;32], StakeInfo>` 改为 `&BTreeMap<[u8;32], u128>`
- **genesis 初始化顺序**: 先构建 WorldState（含 stakes），再从中构建 ValidatorSet

**1.2 SlashingState 持久化到 WorldState**

```rust
// WorldState 新增字段
pub struct WorldState {
    // ... existing fields ...
    pub jailed_validators: BTreeMap<[u8; 32], u64>,    // address → jail_until_height
    pub slash_evidence: Vec<SlashEvidence>,              // 双签证据
    pub validator_missed_slots: BTreeMap<[u8; 32], u64>, // 掉线计数（u64，与 SlashingState 一致）
    pub validator_assigned_slots: BTreeMap<[u8; 32], u64>, // 分配 slot 计数（分母，必须一起持久化）
}
```

SlashingState 从内存 struct 改为 WorldState 的一部分，随 state snapshot 同步。

**⚠️ Borsh 序列化**: 既然主网重置，**删除自定义 `BorshDeserialize` impl**，改用 `#[derive(BorshDeserialize)]`。
消除 19 层 `has_more` 兼容逻辑，杜绝序列化 bug。

**1.3 Slashing 函数签名变更（API 边界）**

```rust
// 之前: slashing.rs 依赖 ValidatorSet
use crate::validator_set::ValidatorSet;
fn slash_stake(validator_set: &mut ValidatorSet, addr: &[u8; 32], basis_points: u64) { ... }

// 之后: slashing 函数直接操作 WorldState.stakes
fn slash_stake(
    stakes: &mut BTreeMap<[u8; 32], u128>,
    addr: &[u8; 32],
    basis_points: u64,
) -> u128 {
    let slashed = stakes[addr] * basis_points as u128 / 10000;
    *stakes.get_mut(addr).unwrap() -= slashed;
    slashed  // 返回燃烧数额
}

// 或者直接接收 &mut WorldState:
fn process_downtime_slashing(state: &mut WorldState, current_height: u64) { ... }
```

**1.4 total_supply() 包含 unbonding_queue + 同步更新审计函数**

```rust
// 之后
pub fn total_supply(&self) -> u128 {
    balances.sum() + stakes.sum() + unbonding_queue.sum()
}
```

**⚠️ 必须同步更新**:
- `chain.rs` 中 `produce_block` 的 supply 完整性检查（~line 501-519）
- `chain.rs` 中 `apply_remote_block_inner` 的 supply 完整性检查（~line 737-755）
- `get_total_supply_audit()` 函数（~line 1190）— 返回值必须与 `total_supply()` 公式一致

### Phase 2: P2P 网络隔离（解决 #5）

**方案**: gossipsub topic **+ request_response 协议** 都加 chain_id

```rust
// gossipsub topic
fn topic_tx(chain_id: &str) -> String { format!("claw/{}/tx/1", chain_id) }
fn topic_block(chain_id: &str) -> String { format!("claw/{}/block/1", chain_id) }
fn topic_vote(chain_id: &str) -> String { format!("claw/{}/vote/1", chain_id) }

// request_response sync 协议（复审新增）
fn sync_protocol(chain_id: &str) -> StreamProtocol {
    StreamProtocol::try_from_owned(format!("/claw/{}/sync/1", chain_id)).unwrap()
}
```

mDNS 仍然可以发现同机节点，但 gossipsub + sync 协议按 chain_id 隔离，消息不会串。

### Phase 3: 共识签名放宽（解决 #6）

**方案**: 对 `apply_remote_block_inner` 的 `is_sync == false` 路径也放宽：

```rust
// 当 active validators < 7 时，只要求 proposer 签名
let required = if active.len() < 7 {
    1  // 早期小网络: proposer 签名即可
} else {
    quorum(active.len())  // 成熟网络: 严格 BFT
};
```

**Vote 收集优化**: 当 `active.len() < 7` 时，跳过 `BroadcastVote` gossip 发送，
减少无效网络流量（收集的 vote 不会被使用）。

### Phase 4: 其他 MEDIUM 修复

**4.1 produce_block / apply_remote_block 竞态（#7）**

在 `produce_block` 中检查是否刚收到同高度的远程 block：

```rust
fn produce_block(inner: &mut ChainInner) -> Option<Block> {
    let now = SystemTime::now()...;
    if now - inner.latest_block.timestamp < 3 && inner.latest_block.validator != inner.validator_address {
        return None;  // 刚收到远程 block，不重复出块
    }
}
```

**4.2 sync 卡住恢复（#8）**

已在 v0.2.6 实现（bootstrap_addrs 30s 重拨）。验证是否覆盖此场景。

**4.3 fast sync 后历史数据缺失（#9）**

固有限制，文档说明即可。

**4.4 light mode 双 redb 实例（#10）**

标记为实验性，后续修复。

---

## 实施顺序

```
Phase 0 (ChangeDelegation)  ← 安全漏洞，独立修复，即时价值
  ↓
Phase 1 (质押状态统一)       ← 最核心，含 slashing 签名变更 + supply 审计同步 + Borsh 简化
  ↓
Phase 2 (P2P 隔离)           ← gossipsub + request_response 都加 chain_id
  ↓
Phase 3 (共识签名)            ← 放宽 + vote 收集优化
  ↓
Phase 4 (其他)                ← 稳定性
  ↓
主网重置 + 全量部署
```

## 主网重置清单

重构完成后需要重置主网：
1. 更新 genesis.json（如有变化）
2. 清除所有节点的 chain.redb
3. 4 个节点全部用新版本启动
4. 重新执行 Owner 委托质押（3 笔 StakeDeposit）
5. 重新执行 ChangeDelegation（Hetzner self-stake → Owner）
6. 验证 total_supply, validators, block time
7. Observer 24h 确认稳定

## 文件改动预估

| 文件 | 改动 |
|------|------|
| crates/state/src/handlers.rs | Phase 0: ChangeDelegation 权限加固 |
| crates/consensus/src/validator_set.rs | Phase 1: 删 candidates，删 compute_weight(StakeInfo) 改为 u128 |
| crates/consensus/src/slashing.rs | Phase 1: 函数签名改为操作 &mut WorldState / &mut stakes |
| crates/state/src/world.rs | Phase 1: 新增 jailed/slash/assigned_slots 字段，删自定义 Borsh，total_supply 加 unbonding |
| crates/node/src/chain.rs | Phase 1: 删 sync_validator_stakes，更新 supply 完整性检查 + get_total_supply_audit |
| crates/p2p/src/protocol.rs | Phase 2: topic 加 chain_id |
| crates/p2p/src/behaviour.rs | Phase 2: topic 动态生成 + sync 协议加 chain_id |
| crates/p2p/src/network.rs | Phase 2: P2pNetwork::new 接收 chain_id |
| crates/node/src/main.rs | Phase 2: 传 chain_id 给 P2pNetwork |
