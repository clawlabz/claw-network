# ClawNetwork 链重置 + 全面修复计划

> 审核日期: 2026-03-21. 确认后执行。

## 为什么重置

当前链存在多个设计缺陷，打补丁已经导致了生产事故（v0.1.35-v0.1.40 连续 6 个版本出 bug）。目前没有真实用户，是最好的重置窗口。

## 一、链上设计缺陷（需在重置前修复）

### P0 — 必须修复

| # | 问题 | 位置 | 说明 |
|---|------|------|------|
| 1 | **Genesis 占位验证者无法移除** | genesis.rs | genesis validators 用 `[30, 0, 0, ...]` 假地址，没人有私钥，无法 unstake |
| 2 | **Fallback validator hack 凭空创建质押** | chain.rs | 节点启动自动加 MIN_STAKE*100，破坏供应量 |
| 3 | **Genesis 分配地址无私钥** | genesis.rs | genesis_address(1-5) 没有私钥，团队份额无法操作 |
| 4 | **节点重启后 validator set 从 genesis 重建** | chain.rs | 不从 state.stakes 恢复，丢失运行时状态 |
| 5 | **State snapshot 验证用 SHA256，state_root 用 Blake3** | sync.rs | 哈希算法不一致 |
| 6 | **StakeClaim handler 未实现** | handlers.rs | unbonding_queue 有数据但无法 claim |
| 7 | **CLW 命名不统一** | 全项目 | 应统一为 CLAW |
| 8 | **Genesis 时间戳硬编码** | genesis.rs | 固定为 2025-03-12，应改为配置项 |

### P1 — 应该修复

| # | 问题 | 位置 | 说明 |
|---|------|------|------|
| 9 | **单验证者无 BFT 签名要求** | election.rs | 1 个验证者时不需要签名，不安全 |
| 10 | **Validator uptime 不按 epoch 重置** | chain.rs | 计数器无限累加，百分比越来越失真 |
| 11 | **Commission 默认值 10000** | rewards.rs | 默认验证者拿 100%，delegator 拿 0%，不直观 |
| 12 | **Platform report epoch 硬编码 100** | handlers.rs | 应用 EPOCH_LENGTH 常量 |
| 13 | **Empty validator set 可能导致永久停链** | validator_set.rs | 所有验证者被 jail 后 active set 为空 |
| 14 | **Genesis config 无版本号** | genesis.rs | 格式变更时旧文件被错误解析 |
| 15 | **Fork recovery 不验证 latest_block 高度** | chain.rs | snapshot 可跳到任意高度 |

## 二、命名不统一（全项目修复）

### Token 命名: CLW → CLAW

| 项目 | 影响范围 | 优先级 |
|------|---------|--------|
| **官网 (clawnetwork-web)** | messages/en.json, zh.json, 所有 .mdx 文档 | P0 用户可见 |
| **钱包扩展** | AmountInput "CLW" → "CLAW", formatCLW 函数名 | P0 用户可见 |
| **Explorer** | formatCLW/formatCLAW 显示文字 | P0 用户可见 |
| **节点 CLI** | "Amount in CLW" → "Amount in CLAW" | P1 |
| **节点 Rust 代码** | CLW_DECIMALS 等常量注释 | P2 内部 |
| **ClawPay SDK** | CLW_DECIMALS → CLAW_DECIMALS | P2 内部 |
| **claw-sdk** | 同上 | P2 内部 |
| **claw-bridge** | 同上 | P2 内部 |

### 旧 CLZ 路由（ClawArena）

| 文件 | 操作 |
|------|------|
| `api/wallet/convert-earned-to-clz/route.ts` | 删除（新 CLAW 路由已存在） |
| `api/wallet/convert-clz-to-topup/route.ts` | 删除 |
| `api/wallet/deposit-clz/route.ts` | 删除 |
| `api/wallet/withdraw-clz/route.ts` | 删除 |
| `lib/clz.ts` | 重命名为 `lib/claw.ts`，更新所有引用 |
| `ClawMarket withdraw enum` | `'CLZ'` → `'CLAW'` |

## 三、新 Genesis 设计

```rust
GenesisConfig {
    chain_id: "claw-mainnet-1",  // 保持不变，旧数据已清空
    timestamp: <部署时的 Unix 时间戳>,
    version: 1,

    allocations: [
        // 团队份额 → Owner Key（有私钥，可操作）
        { address: "71fa1a51...", balance: 15%, label: "team" },  // 1.5 亿 CLAW
        // 其余保持系统地址
        { address: genesis_addr(1), balance: 40%, label: "node_incentives" },
        { address: genesis_addr(2), balance: 25%, label: "ecosystem_fund" },
        { address: genesis_addr(4), balance: 10%, label: "early_contributors" },
        { address: genesis_addr(5), balance: 10%, label: "liquidity_reserve" },
    ],

    validators: [
        // 创世节点 — Hetzner，初始质押 100 万 CLAW（从团队份额扣除）
        { address: "ffa28f7c...", stake: "1000000000000000" },  // 1,000,000 CLAW
    ],
}
```

### 质押规划

| 角色 | 质押额 | 等值估算 ($1/CLAW) | 说明 |
|------|--------|-------------------|------|
| 创世节点 (Hetzner) | 1,000,000 CLAW | $1M | Genesis 写入 |
| 自营节点 (阿里云/MacMini/Win11) | 100,000 CLAW | $100K | Owner Key 委托 |
| 外部验证者 Pioneer | 10,000 CLAW | $10K | 保证金 $2,000 |
| 外部验证者 Partner | 50,000 CLAW | $50K | 保证金 $5,000 |
| 外部验证者 Enterprise | 100,000 CLAW | $100K | 保证金 $10,000 |

### 与旧 genesis 的区别
1. **无占位验证者** — genesis validator 是真实的 Hetzner 地址
2. **团队份额给 Owner Key** — 有私钥可操作（质押、转账、委托）
3. **初始质押从团队余额扣除** — 不再凭空创建
4. **创世节点 100 万质押** — 与估值匹配
5. **timestamp 使用部署时间** — 不再硬编码
6. **增加 version 字段** — 配置格式可演进

## 四、代码修改清单

### 节点代码 (Rust)

| # | 文件 | 改动 |
|---|------|------|
| 1 | `genesis.rs` | 新的 default_mainnet_v2()，真实地址，version 字段，动态 timestamp |
| 2 | `chain.rs` | 移除 fallback validator hack；从 state.stakes 恢复 validator set |
| 3 | `chain.rs` | state snapshot 验证改用 Blake3（或统一用 SHA256） |
| 4 | `chain.rs` | fork recovery 验证 latest_block height 和 state_root |
| 5 | `chain.rs` | validator uptime 每 epoch 重置 |
| 6 | `chain.rs` | pending_votes 在 apply_remote_block 时清空 |
| 7 | `handlers.rs` | 实现 StakeClaim handler（处理 unbonding_queue） |
| 8 | `handlers.rs` | platform report epoch 用 EPOCH_LENGTH 常量 |
| 9 | `rewards.rs` | commission 默认值改为 8000（80%）而非 10000 |
| 10 | `validator_set.rs` | active set 为空时保留至少 1 个验证者 |
| 11 | `election.rs` | 单验证者也要求自签名 |
| 12 | `main.rs` | CLI 文档 "CLW" → "CLAW" |
| 13 | `state.rs` | 增加 NATIVE_TOKEN_SYMBOL = "CLAW" 常量 |

### 前端项目

| # | 项目 | 改动 |
|---|------|------|
| 14 | **钱包扩展** | 所有 UI "CLW" → "CLAW"；formatCLW → formatCLAW |
| 15 | **Explorer** | formatCLW → formatCLAW；显示文字统一 |
| 16 | **官网** | messages/en.json, zh.json 所有 "CLW" → "CLAW" |
| 17 | **官网** | 所有 .mdx 文档 "CLW" → "CLAW" |
| 18 | **ClawArena** | 删除旧 CLZ 路由，重命名 lib/clz.ts |
| 19 | **ClawMarket** | withdraw enum "CLZ" → "CLAW" |

## 五、执行步骤

```
阶段 1：代码修复（在本地开发）
  1.1 修复所有 P0 设计缺陷（#1-#8）
  1.2 修复 P1 设计缺陷（#9-#15）
  1.3 统一命名 CLW → CLAW（#14-#19）
  1.4 本地 devnet 测试：
      - 单节点启动 → 出块 ✓
      - 重启 → 继续出块 ✓
      - 第二节点 fast sync → 同步 ✓
      - 转账 → 到账 ✓
      - 质押/unstake/claim → 全流程 ✓
      - epoch 轮换 → 验证者集更新 ✓
  1.5 全部测试通过 → 发版 v0.2.0

阶段 2：链重置（协调执行）
  2.1 停止所有 4 个节点
  2.2 清空所有节点的 chain.redb（保留 key.json + p2p_key）
  2.3 部署 v0.2.0 到所有节点
  2.4 Hetzner 先启动（genesis validator）→ 确认出块
  2.5 其他节点 fast sync 加入
  2.6 Owner Key 委托质押给 4 个节点

阶段 3：前端部署
  3.1 部署官网更新（命名修正）
  3.2 部署 Explorer 更新
  3.3 重新打包钱包扩展 → Chrome Store 更新
  3.4 E2E 验证：官网/Explorer/钱包/节点 全链路

阶段 4：验证
  4.1 所有节点出块正常
  4.2 转账/质押/unstake 全流程正常
  4.3 Explorer 显示正确
  4.4 钱包连接正常
  4.5 记录 v0.2.0 baseline
```

## 六、风险评估

| 风险 | 概率 | 影响 | 缓解 |
|------|------|------|------|
| 新 genesis 配置错误 | 中 | 需要再次重置 | 本地 devnet 充分测试 |
| 命名漏改 | 低 | 用户看到不一致 | grep 全项目扫描 |
| 节点重启后仍不出块 | 中 | 重复之前的问题 | devnet 测试重启场景 |
| 钱包扩展审核被拒 | 低 | 延迟上架 | 已提交过一次，格式OK |

## 七、不做的事

- 不改架构（共识算法、P2P 协议不变）
- 不改总供应量（1B CLAW 不变）
- 不改 validator set 大小（保持 21）
- 不改奖励公式
- 不改交易类型定义（保留所有 14 种）
- 不改私钥（所有地址保持不变）
