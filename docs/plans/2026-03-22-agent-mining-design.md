# Agent Mining 设计方案 — PoUN (Proof of Useful Node)

> 日期: 2026-03-22
> 状态: 设计确认
> 定位: DPoS 保安全，PoUN 促参与
> 口号: 每个 AI Agent 都是一个节点

---

## 1. 概述

ClawNetwork 的品牌承诺是"每个 AI Agent 都是一个节点"。当前 21 个 DPoS Validator 负责出块和共识安全，但其他 Agent 节点加入网络后无法获得任何收益，缺乏参与动力。

Agent Mining 引入 **PoUN (Proof of Useful Node)** 机制，提供三种挖矿模式，覆盖所有用户群体。共识和挖矿分离 — Validator 出块是骨架，Agent Mining 是血肉。

### 业界参考

| 项目 | 共识 | 挖矿 | 模式 |
|------|------|------|------|
| Helium | Validator (PoS) | Hotspot (Proof of Coverage) | 覆盖即挖矿 |
| Filecoin | Expected Consensus | Proof of Storage | 存储即挖矿 |
| Bittensor | Validator | Miner (AI 推理) | AI 推理即挖矿 |
| Theta | Validator (BFT) | Edge Node (带宽) | 带宽即挖矿 |
| **ClawNetwork** | **Validator (DPoS)** | **Agent (PoUN)** | **多模式挖矿** |

### 与竞品的核心区别

| 维度 | Bittensor | io.net | Ritual | **ClawNetwork** |
|------|-----------|--------|--------|-----------------|
| 定位 | 纯 AI 推理网络 | GPU 租赁市场 | 链上 AI 推理 | AI Agent 公链 |
| 链 | 自有 L1 | 非区块链 | ETH L2 | 自有 L1 |
| 需求来源 | 外部查询 | 外部租户 | DApp 调用 | 开放市场（内外部） |
| 挖矿模式 | 本地模型推理 | GPU 出租 | 本地模型推理 | **三模式：在线+LLM共享+算力** |
| 独有优势 | — | — | — | **LLM 额度共享（零硬件门槛）** |

---

## 2. 架构总览

```
┌──────────────────────────────────────────────────────┐
│                   ClawNetwork                         │
│                                                       │
│  ┌──────────────┐    ┌─────────────────────────────┐ │
│  │  共识层 DPoS  │    │     挖矿层 PoUN             │ │
│  │  21 Validator │    │                             │ │
│  │  出块 + 签名  │    │  Tier 1: 在线挖矿（微量）   │ │
│  │              │    │  Tier 2: LLM 共享（中等）    │ │
│  │  奖励来源:    │    │  Tier 3: 算力挖矿（高）     │ │
│  │  Validator    │    │                             │ │
│  │  Rewards 25%  │    │  奖励来源: Agent Mining     │ │
│  │              │    │  Pool 15%                   │ │
│  └──────────────┘    └─────────────────────────────┘ │
│                                                       │
│  ┌─────────────────────────────────────────────────┐ │
│  │           AI 推理市场（开放）                      │ │
│  │                                                   │ │
│  │  需求方: ClawArena / ClawMarket / 任何外部产品    │ │
│  │  供给方: 所有 Agent Mining 节点                   │ │
│  │  计费: CLAW 付费调用                              │ │
│  └─────────────────────────────────────────────────┘ │
└──────────────────────────────────────────────────────┘
```

---

## 3. 三种挖矿模式

### 3.1 Tier 1: 在线挖矿（人人可参与）

**机制**: 运行全节点 + 保持在线 + 同步区块 = 微量 CLAW 奖励

**目的**: 激励参与度，扩大网络节点数量

**参与条件**:
- 链上注册为 Agent（gas 费 0.001 CLAW，Faucet 提供）
- 运行 claw-node，保持同步
- 无质押要求，0 成本启动

**奖励占比**: Agent Mining Pool 的 20%

### 3.2 Tier 2: LLM 共享（零硬件门槛）

**机制**: 用户将自己的 LLM 订阅（ChatGPT/Claude/DeepSeek 等）闲置额度
共享到网络，按实际调用量获得 CLAW 奖励。

**目的**: 利用全球 AI 用户的闲置 LLM 额度，汇聚成大规模推理能力

**工作流程**:
```
用户配置:
  - 选择共享的 LLM: Claude / GPT-4o / DeepSeek / Gemini
  - 输入 API Key (加密存储在本地，不上链)
  - 设置额度上限: 每小时 N 次请求
  - 设置可用时段: 全天 / 仅闲时
      ↓
网络收到推理请求 → 调度层匹配最优节点 → 通过用户 API Key 调用 → 返回结果
      ↓
用户赚取 CLAW（按调用量 × 模型质量权重）
```

**LLM 共享奖励权重**:

| 模型 | 质量权重 | 说明 |
|------|---------|------|
| GPT-4o / Claude Sonnet | ×2.0 | 顶级商业模型 |
| GPT-4o-mini / Claude Haiku | ×1.2 | 中端模型 |
| DeepSeek V3 | ×1.5 | 高性价比 |
| Gemini Pro | ×1.5 | 多模态能力 |
| 其他兼容模型 | ×0.8 | 基础奖励 |

**奖励占比**: Agent Mining Pool 的 30%

**安全保障**:
- API Key 仅存储在用户本地，加密保存，不上链不外传
- 请求通过代理层转发，不暴露调用方身份
- 节点无法关联请求者与请求内容

### 3.3 Tier 3: 算力挖矿（有 GPU 的用户）

**机制**: 本地部署开源 AI 模型，为网络提供推理服务，按工作量获得 CLAW。

**硬件分级与奖励**:

| 等级 | 模型范围 | 最低硬件 | 奖励权重 |
|------|---------|---------|---------|
| S 级 | 70B+ (Llama 70B, Qwen 72B) | 48GB+ VRAM | ×5.0 |
| A 级 | 13B-34B (CodeLlama 34B, Yi 34B) | 24GB VRAM | ×3.0 |
| B 级 | 7B-13B (Llama 8B, Mistral 7B) | 8GB VRAM | ×1.5 |
| C 级 | 1B-7B (Phi-3, Llama 3B) | CPU / 4GB VRAM | ×1.0 |

**技术实现**:
- claw-node 内置 llama.cpp 推理引擎
- 启动时自动检测 GPU，推荐适合的模型
- 支持模型热切换

**奖励占比**: Agent Mining Pool 的 50%

---

## 4. 奖励分配机制

### 4.1 固定总量模型（参考 BTC）

每个区块的 Agent Mining 奖励总量固定，按权重分配：

```
每块 Agent Mining 奖励: 5 CLAW（第 1 年）

衰减计划:
  Year 1:    5 CLAW/block
  Year 2:    4 CLAW/block
  Year 3:    3 CLAW/block
  Year 4:    2 CLAW/block
  Year 5-10: 1 CLAW/block
  Year 11+:  0.5 CLAW/block
```

### 4.2 权重计算

```
node_reward = block_mining_reward × (node_weight / total_weight)

node_weight = tier_weight × uptime_ratio × reputation_multiplier

tier_weight:
  Tier 3 (算力): model_level_weight × inference_count
  Tier 2 (LLM):  llm_quality_weight × call_count
  Tier 1 (在线): 1.0 × uptime_ratio
```

### 4.3 信誉乘数（Anti-Sybil）

| 节点年龄 | multiplier | 说明 |
|---------|-----------|------|
| 0-7 天 | ×0.1 | 新节点冷启动 |
| 7-30 天 | ×0.5 | 逐步建立信誉 |
| 30 天+ | ×1.0 | 正常收益 |
| 作弊记录 | ×0 | 冻结 30 天 |

### 4.4 防 Sybil

| 防线 | 机制 |
|------|------|
| 固定总量 | 更多节点 = 更少单份收益 |
| 信誉乘数 | 新节点 7 天仅获 10% 奖励 |
| 计算验证 | 定期下发 benchmark 任务验证真实能力 |
| IP 多样性 | 同一 /24 网段最多奖励 3 个节点 |

### 4.5 自动均衡

```
10 个节点分 5 CLAW   → 每节点 ~0.5 CLAW/block
1000 个节点分 5 CLAW → 每节点 ~0.005 CLAW/block

收益 < 运行成本 → 节点自然退出 → 单位收益回升
（与 BTC 矿工经济学一致）
```

---

## 5. 数据隐私

### 5.1 分级隐私

| 级别 | 机制 | 适用场景 | 加价 |
|------|------|---------|------|
| 标准 | TLS 加密传输，节点不留日志 | 通用问答、代码生成 | 无 |
| 增强 | 仅路由到高信誉认证节点 | 商业数据处理 | +20% |
| 机密 | TEE 可信执行环境（中长期） | 涉密数据 | +50% |

### 5.2 安全措施

- 所有请求 TLS 加密传输
- 节点签署隐私协议（链上承诺）
- 推理数据仅在 RAM 中处理，不落盘
- 违规节点：永久踢出 + 没收质押（如有）
- LLM 共享模式：API Key 本地加密存储，不外传

---

## 6. 接入方式

### 6.1 主推：AI Agent 一句话 Prompt 安装

参考 [Agent Reach](https://github.com/Panniantong/Agent-Reach) 的模式 —
用户发给自己的 AI Agent 一句话 prompt + 一个 install.md URL，Agent 读取文档后
自动完成全部安装。

**用户复制这句话给 AI Agent（Claude Code / OpenClaw / Codex / Cursor 等）：**

```
帮我安装 ClawNetwork 挖矿节点：https://raw.githubusercontent.com/clawlabz/claw-miner/main/docs/install.md
```

就这一步。Agent 会自己完成剩下的所有事情。

**install.md 是给 AI Agent 读的安装指引**，内容包括：

```markdown
# ClawNetwork Miner — Installation Guide

## For Humans
复制这句话给你的 AI Agent：
帮我安装 ClawNetwork 挖矿节点：<此文件 URL>

## For AI Agents

### Goal
安装 claw-miner CLI，初始化钱包，注册链上 Agent，启动挖矿。

### Step 1: 检测环境
- 检测 OS（Linux/macOS/Windows）
- 检测 GPU（nvidia-smi / system_profiler）
- 检测可用内存

### Step 2: 安装
pip install clawminer
# 或
npm install -g @clawlabz/claw-miner

### Step 3: 初始化
claw-miner init
# 自动生成钱包、从 Faucet 获取 gas、链上注册 Agent

### Step 4: 根据硬件推荐挖矿模式
- 无 GPU → Tier 1 在线挖矿（默认）
- 有 API Key → 建议 Tier 2 LLM 共享
- 有 GPU ≥ 6GB → 建议 Tier 3 算力挖矿 + 推荐模型

### Step 5: 启动
claw-miner start

### Boundaries
- DO NOT run commands with sudo unless user approved
- DO NOT modify system files outside ~/.claw-miner/
- All config stored in ~/.claw-miner/
```

**安装后，用户在 AI Agent 对话中直接用自然语言操作：**

```
启动挖矿                    → claw-miner start
查看我的收益                  → claw-miner status
共享我的 DeepSeek API 额度    → claw-miner llm add deepseek
部署本地 Llama 8B 模型       → claw-miner model start llama-3.1-8b
停止挖矿                    → claw-miner stop
查看 CLAW 余额               → claw-miner balance
```

**更新也是一句话：**

```
帮我更新 ClawNetwork 挖矿节点：https://raw.githubusercontent.com/clawlabz/claw-miner/main/docs/update.md
```

### 6.2 与 OpenClaw / AI Agent 的关系

OpenClaw 是开源项目，不"内置"任何第三方组件。通过以下方式自然关联：

| 方式 | 说明 |
|------|------|
| OpenClaw Plugin Marketplace | 发布为推荐插件，用户自主选择安装 |
| 安装 Prompt Template | 提供 OpenClaw 专属的一句话安装 prompt |
| Claude Code Plugin | `claude plugin add github:clawlabz/claw-miner` |
| 文档推荐 | OpenClaw 文档中介绍 ClawNetwork 节点（非强制） |

**核心原则**：不强制绑定，用户自主选择。安装后 claw-miner 是独立的 CLI 工具，
可以在任何 AI Agent 中使用，也可以脱离 Agent 独立运行。

### 6.3 兜底：命令行手动安装（不依赖 AI Agent）

```bash
# 一行安装
curl -sL https://get.clawlabz.xyz/miner | sh

# 或 pip
pip install clawminer

# 初始化 + 启动
claw-miner init
claw-miner start
```

支持平台：Linux x86_64 / macOS ARM / Windows

### 6.4 CLI 命令参考

```bash
# 基础
claw-miner init                           # 初始化钱包 + 注册 Agent
claw-miner start                          # 启动（默认在线挖矿）
claw-miner stop                           # 停止
claw-miner status                         # 查看状态/收益/排名
claw-miner balance                        # 查看 CLAW 余额

# LLM 共享（Tier 2）
claw-miner llm add openai                 # 添加 OpenAI API Key
claw-miner llm add deepseek               # 添加 DeepSeek API Key
claw-miner llm add claude                 # 添加 Claude API Key
claw-miner llm limit 50/hour              # 设置每小时请求上限
claw-miner llm list                       # 查看已配置的 LLM
claw-miner llm remove openai              # 移除

# 算力挖矿（Tier 3）
claw-miner model list                     # 查看可部署的模型
claw-miner model start llama-3.1-8b       # 部署并启动模型
claw-miner model start phi-3-mini         # 部署轻量模型
claw-miner model stop                     # 停止本地模型
claw-miner model benchmark                # 测试本机推理性能

# 配置
claw-miner config show                    # 查看当前配置
claw-miner config wallet export           # 导出钱包
claw-miner config wallet import <key>     # 导入已有钱包
```

### 6.5 接入形态总结

| 方式 | 面向用户 | 安装方式 |
|------|---------|---------|
| **AI Agent 一句话安装** | OpenClaw / Claude Code / Codex / Cursor 用户 | 复制 prompt 发给 Agent |
| **命令行安装** | 开发者 / 服务器 | `curl` 或 `pip install` |

只有这两种，不做浏览器扩展（无法调用本地 GPU/CPU 算力）。

---

## 7. AI 推理市场（开放平台）

### 7.1 定位

ClawNetwork 算力池是**开放的 AI 推理市场**，不仅供内部产品使用：

```
需求方（付费调用）:
  - ClawArena NPC 推理
  - ClawMarket 任务处理
  - 任何外部产品 / 开发者 / DApp

供给方（提供算力）:
  - Tier 2 节点（LLM 共享）
  - Tier 3 节点（本地模型）
```

### 7.2 计费模型

```
API 调用 → 按 token 计费（CLAW）
               │
               ├── 70% → 供给节点
               ├── 20% → 协议金库
               └── 10% → 燃烧（通缩）
```

### 7.3 调用方式

```bash
# 标准 OpenAI 兼容 API
curl https://api.clawlabz.xyz/v1/chat/completions \
  -H "Authorization: Bearer claw_xxx" \
  -d '{
    "model": "llama-3.1-8b",
    "messages": [{"role": "user", "content": "Hello"}]
  }'
```

兼容 OpenAI API 格式，任何使用 OpenAI SDK 的项目可一行代码切换：
```python
client = OpenAI(base_url="https://api.clawlabz.xyz/v1", api_key="claw_xxx")
```

---

## 8. 经济模型

### 8.1 Genesis 分配调整

```
Validator Rewards:    25% (2.5 亿 CLAW) — 出块奖励
Agent Mining Pool:    15% (1.5 亿 CLAW) — PoUN 挖矿奖励
  ├── 在线挖矿:  20% (3000 万)
  ├── LLM 共享:  30% (4500 万)
  └── 算力挖矿:  50% (7500 万)
Ecosystem Fund:       25% (2.5 亿)
Team:                 15% (1.5 亿)
Early Contributors:   10% (1 亿)
Liquidity:            10% (1 亿)
```

### 8.2 收入来源

| 来源 | 分配 |
|------|------|
| 交易 gas 费 | 50% proposer, 20% ecosystem, 30% 燃烧 |
| AI 推理 API 费 | 70% 节点, 20% 协议金库, 10% 燃烧 |
| 合约部署费 | 100% 燃烧 |

### 8.3 通缩机制

- Gas 费 30% 燃烧
- API 调用费 10% 燃烧
- 合约部署费 100% 燃烧
- Mining 奖励按年衰减

---

## 9. 实施路线

| 阶段 | 内容 | 时间 | 复杂度 |
|------|------|------|--------|
| **Phase 1** | 在线挖矿 + claw-miner CLI + Agent 注册 | 3-4 周 | 中 |
| **Phase 2** | LLM 共享（API Key 代理 + 调度 + 计费） | 4-6 周 | 中高 |
| **Phase 3** | 算力挖矿（llama.cpp 集成 + 本地模型） | 6-8 周 | 高 |
| **Phase 4** | AI 推理市场（OpenAI 兼容 API + Gateway） | 4 周 | 中 |
| **Phase 5** | 隐私增强（TEE 支持 + 信誉系统完善） | 未来 | 高 |

---

## 10. 已知风险与缓解

| 风险 | 影响 | 缓解 |
|------|------|------|
| LLM API Key 泄露 | 用户经济损失 | 本地加密存储 + 传输加密 + 不上链 |
| 共享额度超限 | 用户被 LLM 供应商封号 | 严格遵守用户设置的额度上限 + 实时监控 |
| 前期节点少，推理质量差 | 用户体验差 | 初期人工补充算力 + 质量评分门槛 |
| Sybil 刷在线挖矿 | 奖励被稀释 | 固定总量 + 信誉乘数 + IP 限制 |
| CLAW 无法定价 | 用户不知收益价值 | CLAW 可直接换取链上 AI 服务 |
| 模型版权 | 法律风险 | 仅支持 Apache/MIT 协议开源模型 |
| LLM 供应商 ToS | 共享 API Key 可能违反条款 | 研究各供应商条款，合规设计 |
