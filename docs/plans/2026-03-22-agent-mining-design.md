# Agent Mining 设计方案 — PoUN (Proof of Useful Node) v2

> 日期: 2026-03-22
> 版本: v2（经 5 轮多维度审查修正）
> 状态: 设计确认
> 定位: DPoS 保安全，PoUN 促参与
> 口号: 每个 AI Agent 都是一个节点

---

## 1. 概述

ClawNetwork 的品牌承诺是"每个 AI Agent 都是一个节点"。当前 21 个 DPoS Validator
负责出块和共识安全，但其他 Agent 节点加入后无法获得任何收益。

Agent Mining 引入 **PoUN (Proof of Useful Node)** 机制，提供三种挖矿模式。
共识和挖矿分离 — Validator 出块是骨架，Agent Mining 是血肉。

### v2 vs v1 关键变更

| 项目 | v1 (原方案) | v2 (修正后) | 原因 |
|------|-----------|-----------|------|
| Tier 2 | 共享商业 LLM API Key（直接代理） | 通过 OpenClaw 共享 LLM 能力 + 本地开源模型 | 改为合规的 OpenClaw 集成模式 |
| 排放模型 | 线性衰减 5/4/3/2/1 | 几何减半 2.5→1.25→0.625 | 原方案第 4 年耗尽 |
| 奖励池 | Validator 25% + Mining 15% 分离 | 统一池 40% 按比例分配 | 分离池耗尽时间不同步 |
| 安装方式 | 仅 CLI + AI Agent prompt | CLI + AI Agent prompt + GUI 桌面应用 | 非技术用户无法上手 |
| Tier 2/3 质押 | 不需要 | 需要最低 100 CLAW | 无质押无法惩罚作弊 |
| Heartbeat | 每 epoch 发 tx（需 gas） | 每 10 epoch 发 tx（免 gas） | gas 费可能超过收益 |
| 推理后端 | 内置 llama.cpp | 支持 Ollama（推荐）+ llama.cpp | 不重造轮子 |

---

## 2. 三种挖矿模式

### 2.1 Tier 1: 在线挖矿（人人可参与）

- **机制**: 运行全节点 + 保持同步 = 微量 CLAW 奖励
- **门槛**: 注册 Agent（Faucet 提供 gas）+ 运行 claw-node
- **质押**: 不需要
- **奖励占比**: 统一池的 15%

### 2.2 Tier 2: LLM 能力共享（零硬件门槛 — 核心创新点）

> **全球独创**：没有任何竞品提供 LLM 额度共享功能。这是 ClawNetwork 的核心差异化优势。

Tier 2 提供**两种共享方式**，用户可选其一或同时启用：

#### 方式 A：通过 OpenClaw 共享 LLM 能力（推荐，合规）

用户的 OpenClaw/AI Agent 已经合法接入了各种 LLM（Claude/GPT/DeepSeek/Gemini 等）。
claw-miner 嵌入 OpenClaw 生态，复用其已有的 LLM 连接，将闲置能力共享到网络。

```
合规模型：
  用户 → 在自己的 OpenClaw 配好 API Key → OpenClaw 合法调用 LLM
    ↓
  claw-miner 作为 OpenClaw 插件运行 → 复用 OpenClaw 的 LLM 连接
    ↓
  用户选择共享闲置额度 → 设置上限 → 网络路由请求到用户的 OpenClaw
    ↓
  用户的 OpenClaw 发出请求（第一方应用调用，非转售）→ 返回结果
    ↓
  用户赚取 CLAW
```

**为什么这是合规的**：
- API Key 始终在用户本地的 OpenClaw 中，claw-miner 不接触 Key
- 请求由用户自己的 OpenClaw 实例发出，和用户正常使用 OpenClaw 无异
- 用户是在自己的应用中运行，选择为他人提供服务是用户的自主决定
- 与 OpenClaw 本身合法接入 50+ LLM 的方式完全一致
- 接入协议中明确告知风险，用户自行承担

**LLM 共享奖励权重**：

| 模型 | 质量权重 | 说明 |
|------|---------|------|
| GPT-4o / Claude Sonnet | ×2.0 | 顶级商业模型 |
| GPT-4o-mini / Claude Haiku | ×1.2 | 中端模型 |
| DeepSeek V3 / R1 | ×1.5 | 高性价比 |
| Gemini Pro | ×1.5 | 多模态 |
| 其他兼容模型 | ×0.8 | 基础奖励 |

#### 方式 B：本地开源模型共享（有硬件要求）

用户本地运行开源模型（Llama/DeepSeek-R1/Mistral/Phi 等），
通过 Ollama 或 llama.cpp 提供推理端点。

**开源模型分级**：

| 等级 | 模型 | 最低硬件 | 奖励权重 |
|------|------|---------|---------|
| S | Llama 70B, Qwen 72B, DeepSeek-R1 67B | 48GB+ VRAM | ×5.0 |
| A | CodeLlama 34B, Yi 34B | 24GB VRAM | ×3.0 |
| B | Llama 8B, Mistral 7B, DeepSeek-R1 8B | 8GB VRAM | ×1.5 |
| C | Phi-3 Mini, Llama 3B, Qwen 1.5B | CPU / 4GB VRAM | ×1.0 |

#### Tier 2 通用配置

- **门槛**: 方式 A 无硬件要求（只需 OpenClaw + LLM 订阅）；方式 B 需能跑 1B+ 模型
- **质押**: 需要 100 CLAW（可通过 Tier 1 挖到）
- **奖励占比**: 统一池的 35%
- **安全**: API Key 永远不离开用户本地，claw-miner 不存储不传输 Key

### 2.3 Tier 3: 专业算力挖矿（高性能节点）

- **机制**: 持续提供高可用推理服务（99%+ 在线率），支持多模型并发
- **门槛**: 专用 GPU 服务器（24GB+ VRAM）
- **质押**: 需要 1000 CLAW
- **奖励占比**: 统一池的 50%
- **额外收入**: AI 推理市场 API 调用费分成

---

## 3. 奖励经济模型（v2 修正）

### 3.1 统一排放池

**不再拆分** Validator 和 Mining 为两个独立池。
从同一个 Node Incentive Pool（40% = 4 亿 CLAW）统一排放：

```
每块总排放 = base_reward（随年份衰减）

分配比例:
  Validator (出块者): 65% of base_reward
  Agent Mining:       35% of base_reward
    ├── Tier 1 (在线): 15% × 35% = 5.25%
    ├── Tier 2 (模型): 35% × 35% = 12.25%
    └── Tier 3 (专业): 50% × 35% = 17.50%
```

### 3.2 几何减半（BTC 模型）

```
减半周期: 每 2 年（~21,024,000 blocks）

Year 1-2:   base_reward = 8 CLAW/block
Year 3-4:   base_reward = 4 CLAW/block
Year 5-6:   base_reward = 2 CLAW/block
Year 7-8:   base_reward = 1 CLAW/block
Year 9-10:  base_reward = 0.5 CLAW/block
Year 11+:   base_reward = 0.25 CLAW/block (尾部排放，永不归零)
```

**池消耗计算**:

| 年份 | CLAW/block | 年排放 | 累计 |
|------|-----------|--------|------|
| 1-2 | 8 | 168,192,000 | 168,192,000 |
| 3-4 | 4 | 84,096,000 | 252,288,000 |
| 5-6 | 2 | 42,048,000 | 294,336,000 |
| 7-8 | 1 | 21,024,000 | 315,360,000 |
| 9-10 | 0.5 | 10,512,000 | 325,872,000 |

**池在第 10 年仍有 7400 万 CLAW 余额**（400M - 326M）。
尾部排放 0.25 CLAW/block 可再持续 ~28 年。总续航 **38 年+**。

### 3.3 Agent Mining 权重

```
node_weight = tier_weight × model_weight × uptime_ratio × reputation_multiplier

Tier 1: tier_weight = 1
Tier 2: tier_weight = 5 × model_level_weight(S/A/B/C)
Tier 3: tier_weight = 10 × model_level_weight × availability_bonus

reputation_multiplier:
  Day 0-7:   ×0.2
  Day 7-30:  ×0.5
  Day 30+:   ×1.0
  作弊记录:   ×0，冻结 30 天
```

### 3.4 防 Sybil

| 防线 | Tier 1 | Tier 2 | Tier 3 |
|------|--------|--------|--------|
| 固定总量 | ✓ | ✓ | ✓ |
| 信誉冷启动 | ✓ (×0.2) | ✓ (×0.2) | ✓ (×0.2) |
| 质押要求 | — | 100 CLAW | 1000 CLAW |
| 推理验证 | — | Benchmark 挑战 | Benchmark + 多节点共识 |
| IP 多样性 | /24 限 3 个 | /24 限 3 个 | /24 限 3 个 |
| Heartbeat | 每 10 epoch | 每 10 epoch | 每 epoch |

### 3.5 AI 推理市场收入

```
外部用户调用 API → 按 token 付费（CLAW）
                 │
                 ├── 70% → 供给节点
                 ├── 20% → 协议金库
                 └── 10% → 燃烧（通缩）
```

**定价策略**: 比 OpenAI/DeepSeek 便宜 30-50%，竞争优势在成本。

---

## 4. 数据隐私

### 分级隐私

| 级别 | 机制 | 适用场景 |
|------|------|---------|
| 标准 | TLS 传输 + 不落盘 + 质押惩罚 | 通用问答、代码生成 |
| 增强 | 高信誉认证节点 + 审计日志 | 商业数据 |
| 机密 | TEE 可信执行环境（中长期） | 涉密数据 |

**坦诚声明**: 标准级别下，节点在推理过程中技术上可以读取 prompt 内容。
这与 Bittensor、io.net 等同类网络一致。质押机制提供经济惩罚作为威慑。

---

## 5. 接入方式

### 5.1 入口 1（主推）：AI Agent 一句话 Prompt

参考 [Agent Reach](https://github.com/Panniantong/Agent-Reach) 模式。

**用户复制给 AI Agent**:
```
帮我安装 ClawNetwork 挖矿节点：https://raw.githubusercontent.com/clawlabz/claw-miner/main/docs/install.md
```

install.md 是给 AI Agent 读的安装指引，Agent 自动完成全部安装。

**安装后在对话中操作**:
```
启动挖矿
部署 Llama 8B 模型
查看收益
停止挖矿
```

### 5.2 入口 2（兜底）：命令行

```bash
pip install clawminer
claw-miner init
claw-miner start
```

### 5.3 入口 3（面向非技术用户，中期）：GUI 桌面应用

基于 Tauri 的轻量桌面应用：
- 一键安装（.dmg / .exe）
- 可视化 Dashboard（收益、状态、排名）
- 鼠标操作选择模型 + 启动挖矿
- 参考 Grass 的极简 UX

### 5.4 CLI 命令参考

```bash
# 基础
claw-miner init                           # 初始化钱包 + 注册
claw-miner start                          # 启动在线挖矿
claw-miner stop                           # 停止
claw-miner status                         # 状态/收益/排名
claw-miner balance                        # CLAW 余额

# 开源模型共享 (Tier 2)
claw-miner model list                     # 查看可用模型
claw-miner model start llama-3.1-8b       # 部署模型（自动下载）
claw-miner model start --backend ollama   # 使用已有 Ollama
claw-miner model stop                     # 停止
claw-miner model benchmark                # 测试性能

# 配置
claw-miner config show
claw-miner config wallet export
```

### 5.5 与 OpenClaw / AI Agent 的关系

- OpenClaw Plugin Marketplace 中推荐，用户自主选择安装
- Claude Code Plugin: `claude plugin add github:clawlabz/claw-miner`
- **不强制绑定**，任何 AI Agent 都能一句话安装

---

## 6. 推理验证

| 方案 | 阶段 | 机制 |
|------|------|------|
| 挑战-响应 | Phase 1 | Validator 定期发已知答案的 prompt，比对节点输出 |
| 多节点共识 | Phase 2 | 同一请求发 2-3 节点，结果投票，异常节点扣信誉 |
| TEE 证明 | Phase 3+ | 硬件级可信执行环境（Intel SGX / AMD SEV） |

ZKML 不成熟（当前仅支持小模型，LLM 级别要到 2027+），暂不考虑。

---

## 7. 技术架构

### 7.1 技术栈

| 组件 | 语言/框架 | 理由 |
|------|----------|------|
| claw-miner CLI | Python | AI 生态原生，pip 分发 |
| 推理后端 | Ollama（推荐）/ llama.cpp | 不重造轮子，成熟稳定 |
| 桌面 GUI | Tauri (Rust + Web) | 轻量跨平台 |
| 链上合约 | Rust (claw-node) | 已有基础设施 |
| API Gateway | Python (FastAPI) | 中心化 MVP，后续去中心化 |

### 7.2 架构图

```
┌──────────────────────────────────────────┐
│  用户 / AI Agent                          │
│  "帮我安装挖矿节点" / pip install          │
└───────────────┬──────────────────────────┘
                │
        ┌───────▼───────┐
        │  claw-miner   │  Python CLI
        │  (init/start) │
        └───┬───┬───┬───┘
            │   │   │
     ┌──────┘   │   └──────┐
     ▼          ▼          ▼
  Tier 1     Tier 2     Tier 3
  在线同步    Ollama     GPU Farm
             端口共享    多模型服务
     │          │          │
     └──────┬───┘──────────┘
            │
   ┌────────▼────────┐
   │  ClawNetwork    │  Rust claw-node
   │  (链上注册/奖励) │
   └────────┬────────┘
            │
   ┌────────▼────────┐
   │  API Gateway    │  FastAPI
   │  (推理市场入口)  │  api.clawlabz.xyz
   └─────────────────┘
```

---

## 8. 实施路线

| 阶段 | 内容 | 时间 | 前置条件 |
|------|------|------|---------|
| **Phase 1** | Tier 1 在线挖矿 MVP | 3-4 周 | — |
| | claw-miner CLI (init/start/status) | | |
| | 链上 MinerRegister + Heartbeat | | |
| | 统一排放 + 挖矿奖励分配 | | |
| | install.md + pip 包发布 | | |
| **Phase 2** | Tier 2 开源模型共享 | 4-6 周 | Phase 1 |
| | Ollama 集成 + 模型管理 | | |
| | 推理请求路由 + 节点发现 | | |
| | 挑战-响应验证 | | |
| **Phase 3** | AI 推理市场 | 4 周 | Phase 2 |
| | OpenAI 兼容 API Gateway | | |
| | Token 计费 + 收益分配 | | |
| | Explorer 挖矿数据展示 | | |
| **Phase 4** | GUI 桌面应用 | 3 周 | Phase 1 |
| | Tauri 应用 (.dmg/.exe) | | |
| | 可视化 Dashboard | | |
| **Phase 5** | 增强安全 + 去中心化 | 持续 | Phase 3 |
| | TEE 支持 | | |
| | P2P 路由替代中心化 Gateway | | |
| | 多节点推理共识 | | |

---

## 9. 已知风险

| 风险 | 严重度 | 缓解 |
|------|--------|------|
| 前期 CLAW 无市场价格 | HIGH | 先用 CLAW 换取链上 AI 服务（内循环） |
| GPU 挖矿前期不盈利 | MEDIUM | 定位长期投资，前期靠 Tier 1 低成本获客 |
| 推理质量不稳定 | MEDIUM | 挑战-响应验证 + 信誉系统 |
| Prompt 隐私 | MEDIUM | 坦诚声明 + 质押惩罚 + 中期 TEE |
| 开源模型版权 | LOW | 仅支持 Apache/MIT 许可模型 |

---

## 10. 审查记录

本方案经过 5 轮多维度审查：

| 轮次 | 角度 | 关键发现 | 修正 |
|------|------|---------|------|
| R1 | 竞品调研 | 10 个项目对比 | 确认三层模式独特性 |
| R2 | 技术可行性 | LLM ToS 违规（直接代理）、ZKML 不成熟 | 改为通过 OpenClaw 合规共享 + 本地开源模型 |
| R3 | 经济模型 | 池第 4 年耗尽、15.8% 年通胀 | 改几何减半、统一池 |
| R4 | UX 体验 | 非技术用户 2/10、需求端缺失 | 增 GUI、设计需求端 |
| R5 | 实施评估 | MVP 定义、技术栈选择 | Python + Ollama 方案 |
