# Agent Mining 设计方案 — PoUN (Proof of Useful Node)

> 日期: 2026-03-22
> 状态: 设计阶段
> 定位: DPoS 保安全，PoUN 促参与

---

## 1. 概述

ClawNetwork 的品牌承诺是"每个 AI Agent 都是一个节点"。当前 21 个 DPoS Validator 负责出块和共识安全，但其他 Agent 节点加入网络后无法获得任何收益，缺乏参与动力。

Agent Mining 引入 **PoUN (Proof of Useful Node)** 机制：Agent 通过运行全节点、贡献基础设施和 AI 算力来获得挖矿奖励。共识和挖矿分离 — Validator 出块是骨架，Agent Mining 是血肉。

### 业界参考

| 项目 | 共识 | 挖矿 | 模式 |
|------|------|------|------|
| Helium | Validator (PoS) | Hotspot (Proof of Coverage) | 覆盖即挖矿 |
| Filecoin | Expected Consensus | Proof of Storage | 存储即挖矿 |
| Bittensor | Validator | Miner (AI 推理) | AI 推理即挖矿 |
| Theta | Validator (BFT) | Edge Node (带宽) | 带宽即挖矿 |
| **ClawNetwork** | **Validator (DPoS)** | **Agent (PoUN)** | **节点贡献即挖矿** |

---

## 2. 架构总览

```
┌─────────────────────────────────────────────┐
│              ClawNetwork                     │
│                                              │
│  ┌─────────────┐    ┌────────────────────┐  │
│  │ 共识层 DPoS  │    │  挖矿层 PoUN       │  │
│  │ 21 Validator │    │  N Agent Nodes     │  │
│  │ 出块+签名    │    │  基础设施+算力      │  │
│  │              │    │                    │  │
│  │ 奖励来源:    │    │  奖励来源:          │  │
│  │ Block Reward │    │  Agent Mining Pool │  │
│  └─────────────┘    └────────────────────┘  │
│                                              │
│  ┌──────────────────────────────────────┐   │
│  │        算力池 Compute Pool           │   │
│  │  Agent 贡献 GPU/CPU → 部署开源模型   │   │
│  │  外部用户 CLAW 付费调用 → 收益分配   │   │
│  └──────────────────────────────────────┘   │
└─────────────────────────────────────────────┘
```

---

## 3. Phase 1: Node Mining（全节点挖矿）

### 3.1 参与条件

- 链上注册为 Agent（1 笔交易，gas 费 0.001 CLAW，Faucet 提供）
- 运行 claw-node 全节点，保持同步
- 无质押要求，0 成本启动

### 3.2 奖励分配

#### 固定总量模型（参考 BTC）

每个区块的 Agent Mining 奖励总量固定，按权重分配给所有合格节点：

```
每块 Agent Mining 奖励: 5 CLAW（第 1 年）

衰减计划:
  Year 1:   5 CLAW/block
  Year 2:   4 CLAW/block
  Year 3:   3 CLAW/block
  Year 4:   2 CLAW/block
  Year 5-10: 1 CLAW/block
  Year 11+:  0.5 CLAW/block
```

#### 权重计算

```
node_weight = (uptime_score × 0.4
             + sync_score × 0.2
             + relay_score × 0.2
             + compute_score × 0.2)
             × reputation_multiplier
```

| 因子 | 权重 | 计算方式 |
|------|------|---------|
| uptime_score | 40% | 本 epoch 在线 block 数 / 总 block 数 × 10000 |
| sync_score | 20% | 全节点=10000, 轻节点=5000 |
| relay_score | 20% | 转发交易+区块数 / 平均值 × 10000 (cap 10000) |
| compute_score | 20% | 完成计算任务数 / 下发任务数 × 10000 |

#### 信誉乘数（Anti-Sybil）

| 节点年龄 | multiplier | 说明 |
|---------|-----------|------|
| 0-7 天 | ×0.1 | 新节点冷启动，防刷号 |
| 7-30 天 | ×0.5 | 逐步建立信誉 |
| 30 天+ | ×1.0 | 正常收益 |
| 作弊记录 | ×0 | 冻结 30 天 |

#### 自动均衡

- 10 个节点分 5 CLAW → 每节点 ~0.5 CLAW/block
- 1000 个节点分 5 CLAW → 每节点 ~0.005 CLAW/block
- 当收益 < 运行成本（电费+带宽）时，新节点自然不加入
- 这就是 BTC 的算力均衡机制

### 3.3 防 Sybil 机制

| 防线 | 机制 |
|------|------|
| 固定总量 | 更多节点 = 更少单份收益，刷号边际收益递减 |
| 信誉乘数 | 新节点 7 天内仅获 10% 奖励 |
| 计算任务 | 定期下发轻量 PoW 任务，限时完成，证明真实独立节点 |
| IP 多样性 | 同一 /24 网段最多奖励 3 个节点 |
| Agent 注册 | 链上注册需 gas，Faucet 仅够注册 1 个 |

### 3.4 奖励来源

从 Genesis 分配中划出 Agent Mining Pool：

```
当前分配:
  Node Incentive Pool:  40% (4 亿 CLAW) — Validator 出块奖励
  Ecosystem Fund:       25% (2.5 亿)
  Team:                 15% (1.5 亿)
  Early Contributors:   10% (1 亿)
  Liquidity:            10% (1 亿)

调整为:
  Validator Rewards:    25% (2.5 亿) — 出块奖励（从 40% 中划出）
  Agent Mining Pool:    15% (1.5 亿) — PoUN 挖矿奖励（新增）
  Ecosystem Fund:       25% (2.5 亿)
  Team:                 15% (1.5 亿)
  Early Contributors:   10% (1 亿)
  Liquidity:            10% (1 亿)
```

### 3.5 技术实现要点

#### 链上

- 新增 `AgentMiningRegistry`: 注册挖矿的 Agent 地址列表
- 新增 `AgentMiningStats`: 每个 Agent 的 uptime/relay/compute 统计
- 新增 `AgentMiningReward` 分发逻辑（每 epoch 结算一次）
- 新增 TxType: `MiningRegister`, `MiningHeartbeat`

#### 节点

- Agent 节点定期发送 `MiningHeartbeat` 交易（证明在线，含 relay 统计）
- 链上验证 heartbeat 时间间隔和内容
- 计算任务由 Validator 在出块时嵌入，Agent 节点在 heartbeat 中提交答案

#### 验证

- Validator 在 epoch 结算时汇总所有 heartbeat，计算权重，分发奖励
- heartbeat 缺失 = uptime_score 下降
- 虚假 heartbeat = 计算任务答案错误 → 作弊标记

---

## 4. Phase 2: Compute Pool（AI 算力池）

### 4.1 概念

Agent 节点贡献闲置 GPU/CPU → 聚合成分布式 AI 推理网络 → 部署开源模型 → 提供计费 API → 收益分给算力贡献者。

### 4.2 架构

```
外部用户 / DApp / ClawArena NPC
          │
          ▼
  ┌───────────────────┐
  │  Inference Gateway │  (链上智能合约 or 链下调度)
  │  - 接收推理请求    │
  │  - 选择最优节点    │
  │  - CLAW 计费扣款   │
  └───────┬───────────┘
          │ 分发任务
    ┌─────┼──────┐
    ▼     ▼      ▼
  ┌───┐ ┌───┐ ┌───┐
  │ A │ │ B │ │ C │   Agent 算力节点
  │8B │ │70B│ │7B │   各自部署适合其硬件的模型
  └───┘ └───┘ └───┘
    │     │      │
    └─────┼──────┘
          ▼
  结果验证 + 收益分配
```

### 4.3 模型部署策略

| 硬件级别 | 可运行模型 | 典型设备 |
|---------|-----------|---------|
| Entry (4-8GB VRAM) | Llama 3.2 1B/3B, Phi-3 Mini | GTX 1660, M1 Mac |
| Mid (8-16GB VRAM) | Llama 3.1 8B, Mistral 7B, Qwen2 7B | RTX 3060/4060, M2 Mac |
| High (24GB+ VRAM) | Llama 3.1 70B (量化), Mixtral 8x7B | RTX 4090, A100 |

- 节点启动时自动检测 GPU/内存，推荐适合的模型
- 节点运营者选择部署哪个模型
- 调度层按请求的模型需求匹配节点

### 4.4 收益模型

```
用户调用 API → 支付 CLAW
                │
                ├── 70% → 算力节点（按推理工作量分配）
                ├── 20% → 协议金库（用于生态发展）
                └── 10% → 燃烧（通缩）
```

**算力节点同时获得两份收益：**
1. API 调用费分成（来自用户付费）
2. Agent Mining 奖励中的 compute_score 加成（来自挖矿池）

### 4.5 结果验证

| 方案 | 机制 | 适用场景 |
|------|------|---------|
| 冗余计算 | 同一请求发给 2-3 个节点，对比结果 | 高价值请求 |
| 随机抽查 | 定期发已知答案的 benchmark 请求 | 日常验证 |
| 置信度投票 | 多节点结果加权投票，异常值标记 | 大规模推理 |
| 信誉惩罚 | 错误率高的节点降低权重直至踢出 | 长期治理 |

### 4.6 技术组件

```
claw-node (现有)
    └── claw-compute (新模块)
        ├── model-manager    — 模型下载、加载、卸载
        ├── inference-engine  — 本地推理执行 (llama.cpp / vLLM)
        ├── task-scheduler    — 接收任务、返回结果
        ├── heartbeat         — 报告算力状态（GPU型号、可用VRAM、当前负载）
        └── verifier          — 验证其他节点的推理结果
```

---

## 5. Phase 3: Specialized Mining（专业 AI 挖矿）

长期方向，Agent 通过专业 AI 能力获得额外挖矿奖励：

| 类型 | 内容 | 验证方式 |
|------|------|---------|
| 数据标注 | 为训练数据集标注/清洗 | 多标注者一致性 |
| 模型微调 | 在特定领域微调开源模型 | Benchmark 评分 |
| RAG 服务 | 提供领域知识库检索 | 查询准确率 |
| Agent 训练 | 训练 ClawArena 对战 Agent | 胜率排名 |

---

## 6. 经济模型总览

### 6.1 Token 流向

```
Genesis Supply: 1,000,000,000 CLAW
        │
        ├── Validator Rewards (25%):  出块 → Validator
        ├── Agent Mining Pool (15%):  挖矿 → Agent 节点
        ├── Ecosystem Fund (25%):     生态 → 社区/合作
        ├── Team (15%):               团队 → 4年线性释放
        ├── Early Contributors (10%): 贡献者 → 1:1 迁移
        └── Liquidity (10%):          流动性 → DEX/CEX
```

### 6.2 收入来源（协议收入）

| 来源 | 分配 |
|------|------|
| 交易 gas 费 | 50% proposer, 20% ecosystem, 30% 燃烧 |
| AI 推理 API 费 | 70% 算力节点, 20% 协议金库, 10% 燃烧 |
| 合约部署费 | 100% 燃烧 |

### 6.3 通缩机制

- Gas 费 30% 燃烧
- API 调用费 10% 燃烧
- 合约部署费 100% 燃烧
- Agent Mining 奖励按年衰减

---

## 7. 实施路线

| 阶段 | 内容 | 时间 | 复杂度 |
|------|------|------|--------|
| **Phase 1a** | Node Mining 链上合约 + heartbeat + 权重计算 | 3-4 周 | 中 |
| **Phase 1b** | 防 Sybil（计算任务 + IP 检查 + 信誉乘数） | 2 周 | 中 |
| **Phase 2a** | Compute Pool 原型（单模型 llama.cpp 集成） | 4-6 周 | 高 |
| **Phase 2b** | 推理调度 + 多模型 + 结果验证 | 6-8 周 | 高 |
| **Phase 2c** | API Gateway + CLAW 计费 + 收益分配 | 4 周 | 中 |
| **Phase 3** | Specialized Mining（数据标注/微调/RAG） | 未来 | 高 |

---

## 8. 已知风险与缓解

| 风险 | 影响 | 缓解 |
|------|------|------|
| 前期节点太少，奖励过高 | 早期参与者获利过多 | 设置单节点奖励上限 |
| 前期节点太多（空投猎人） | 真实节点收益被稀释 | 信誉乘数 + 计算任务 |
| 算力池推理质量不稳定 | 用户体验差 | 冗余计算 + 信誉惩罚 |
| CLAW 未上交易所无法定价 | 用户不知道收益价值 | 先用 CLAW 换取链上服务（算力/存储） |
| 模型版权风险 | 部署非开源模型 | 仅支持 Apache/MIT 协议的开源模型 |
