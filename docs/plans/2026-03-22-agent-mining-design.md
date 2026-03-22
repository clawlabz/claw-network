# Agent Mining 设计方案 v3

> 日期: 2026-03-22
> 版本: v3（经 10 轮多维度审查修正）
> 状态: 设计确认
> 定位: DPoS 保安全，Agent Mining 促参与
> 口号: 每个 AI Agent 都是一个节点
> 审查笔记: [review-notes.md](./2026-03-22-agent-mining-review-notes.md)

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

#### 方式 A：共享 AI Agent 的 LLM 能力（零硬件门槛）

claw-miner 可接入**任何 AI Agent 工具**，复用其已有的 LLM 连接，
将用户闲置的 LLM 额度共享到网络。

**支持的接入方式**：

| AI Agent / 工具 | 可共享的 LLM | 接入方式 |
|----------------|-------------|---------|
| **OpenClaw** | 50+ 种 LLM（Claude/GPT/DeepSeek/Gemini 等） | 插件集成 |
| **Claude Code** | Claude 系列 | CLI 插件 |
| **Codex** | OpenAI 系列 | CLI 集成 |
| **Gemini CLI** | Gemini 系列 | CLI 集成 |
| **自定义** | 任意 OpenAI 兼容 API | 手动配置 endpoint + key |

**技术实现（v3 明确）**：

claw-miner 是**独立进程**，不拦截宿主 Agent 的内部调用。
它读取用户配置的 LLM API Key（或环境变量），直接调用 LLM API。
与 OpenClaw 使用 API Key 的方式完全一致。

```
架构：
  [claw-miner 进程] ←─ WebSocket 长连接 ─→ [API Gateway]
         │                                    (接收推理请求)
         │ 读取用户配置的 API Key
         ▼
  [LLM Provider API]  (OpenAI / Anthropic / DeepSeek / Google)
         │
         └─→ 返回结果 → claw-miner → Gateway → 调用方
```

**不依赖宿主 Agent 的内部机制**：
- 不需要 Claude Code 的"插件系统"来拦截请求
- 不需要 Codex 的内部 API 钩子
- claw-miner 就像 OpenClaw 一样，是一个独立的 LLM 调用者
- 用户在 claw-miner 中配置 Key，claw-miner 直接调用

**并发保护**：
- 用户设置每小时限额（如 50 次），claw-miner 严格遵守
- 预留用户自用额度：claw-miner 只使用用户设定的共享额度
- 如果 LLM 返回限流错误，claw-miner 自动暂停并通知 Gateway

**LLM 共享奖励权重**：

| 模型级别 | 示例 | 质量权重 |
|---------|------|---------|
| 顶级 | GPT-4o, Claude Sonnet/Opus, Gemini Ultra | ×2.0 |
| 高端 | DeepSeek V3/R1, GPT-4o-mini, Claude Haiku | ×1.5 |
| 中端 | Gemini Flash, Mistral Large | ×1.2 |
| 入门 | 其他兼容模型 | ×0.8 |

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

## 10. 链上升级过渡计划

### 现状

当前链运行排放计划：10/8/6/4/2/1 CLAW/block（代码在 `rewards.rs`）。
链高度 ~1300（极早期），已排放约 13,000 CLAW（可忽略）。

### 过渡方案

在 Phase 1 实施时，通过链上升级（硬分叉）切换排放计划：

```
升级前: 10 CLAW/block → 100% 给 Validator
升级后: 8 CLAW/block → 65% Validator (5.2) + 35% Agent Mining (2.8)
```

**Validator 收入变化**：从 10 降到 5.2 CLAW/block（-48%）。
这在当前阶段可接受，因为：
- 链极早期（height ~1300），影响范围小
- 所有 4 个 Validator 都是 ClawLabz 自有节点
- Agent Mining 为网络带来的增长价值远超 Validator 收入减少
- 未来外部 Validator 加入时，基于新排放计划

---

## 11. 需求端设计

### 11.1 API 消费者入口

```
开发者 → api.clawlabz.xyz 注册 → 获取 API Key → 充值 CLAW → 调用推理
```

### 11.2 CLAW 获取方式（无交易所阶段）

| 方式 | 说明 |
|------|------|
| Faucet | 免费获取少量 CLAW（够试用） |
| 自己挖矿 | 运行 Tier 1 节点赚取 |
| OTC | 与现有持有者场外交易 |
| 协议销售 | 从 Ecosystem Fund 按固定价售出（bootstrap 阶段） |

### 11.3 定价

以 USD 定价，CLAW 结算。避免 CLAW 波动导致成本不可预测：

```
推理定价: $X / 1M tokens (USD 定价)
结算: 按当前 CLAW/USD 汇率折算（初期由协议提供参考价）
```

---

## 12. 治理

### 初期: ClawLabz Multisig

- 3/5 多签控制参数调整
- 参数变更需提前 7 天公示
- 所有变更记录在链上

### 中期: 链上投票

- CLAW 持有者按持有量投票
- 提案门槛: 10,000 CLAW
- 投票周期: 7 天
- 通过门槛: >50% 参与 + >66% 赞成

---

## 13. 安全防御（针对 R9 攻击推演）

### 13.1 防 Fake Tier 2（质量欺诈）

**盲挑战机制**：验证请求通过真实用户通道发送，矿工无法区分挑战和正常请求。
5% 的用户请求被静默复制到第二个节点，比对质量评分。

### 13.2 防 Heartbeat 博弈

**随机 Spot Check**：除定期 heartbeat 外，Validator 随机发送存活探测。
错过 spot check 扣信誉，连续 3 次错过标记为离线。

**累积在线证明**：节点每个 block 签名最新 block hash，形成哈希链。
heartbeat 提交时附带哈希链，缺失 block 说明当时离线。

### 13.3 防 Sybil Tier 1

**Tier 1 微量质押**：10 CLAW（可通过 Faucet 或 1 天挖矿获得），
使大规模 Sybil 有前置成本。

**次线性奖励**：同一资金来源的节点组，奖励按 `sqrt(n)` 缩放而非线性。

---

## 14. 成功指标

### Phase 1 KPIs（3-4 周后评估）

| 指标 | 目标 | 说明 |
|------|------|------|
| 活跃 Tier 1 矿工 | ≥ 50 | 不含 Sybil |
| 矿工 30 天留存率 | ≥ 60% | |
| Heartbeat 成功率 | ≥ 95% | 网络健康度 |
| 挖矿奖励正确分发 | 100% | 无 bug |
| 无安全事件 | 0 | |

### Phase 1 → Phase 2 过渡条件

- Phase 1 运行 ≥ 30 天无 CRITICAL bug
- ≥ 30 个独立 Tier 1 矿工
- 社区反馈正面

---

## 15. GTM（获客计划）

### 首批 1000 矿工来源

| 渠道 | 目标 | 方式 |
|------|------|------|
| OpenClaw 社区 | 200 | 插件推荐 + 教程 |
| Claude Code / Codex 用户 | 300 | `claude plugin add` + AI 社区推广 |
| r/LocalLLaMA + HuggingFace | 200 | Ollama 用户精准触达 |
| Crypto Twitter | 200 | KOL 合作 + 空投预期 |
| YouTube 教程 | 100 | 挖矿教程视频 |

### 推荐机制

- 推荐人获得被推荐人 10% 的挖矿收益加成（持续 90 天）
- 被推荐人免去 7 天冷启动（直接 ×0.5 起步）

---

## 16. 审查记录

本方案经过 **10 轮**多维度审查：

| 轮次 | 角度 | 关键发现 | 修正 |
|------|------|---------|------|
| R1 | 竞品调研 | 10 个项目对比 | 确认三层模式独特性 |
| R2 | 技术可行性 | ZKML 不成熟 | 用挑战-响应验证 |
| R3 | 经济模型 | 池第 4 年耗尽 | 改几何减半，38 年续航 |
| R4 | UX 体验 | 非技术用户 2/10 | 增 GUI 桌面应用 |
| R5 | 实施评估 | 技术栈选择 | Python + Ollama |
| R6 | 经济数学精算 | 排放计划与代码冲突 | 明确过渡升级方案 |
| R7 | Tier 2 技术流 | 不是插件拦截，是独立进程 | 重写技术架构 |
| R8 | 文档完整性 | 55 个缺口 | 增加需求端/治理/迁移/KPI 章节 |
| R9 | 攻击推演 | 3 大攻击向量 | 增加盲挑战/spot check/微质押 |
| R10 | 市场定位 | PoUN 命名差、无 GTM | 改为 Agent Mining + 增 GTM |

详细审查笔记: [review-notes.md](./2026-03-22-agent-mining-review-notes.md)
