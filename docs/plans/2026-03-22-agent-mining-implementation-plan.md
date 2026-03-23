# Agent Mining Phase 1 — Implementation Plan

> 日期: 2026-03-22
> 基于: [agent-mining-design.md](./2026-03-22-agent-mining-design.md) v3
> 方法: TDD（测试先行）+ 原子化模块 + 多 Agent 并行
> 预期: 3-4 周完成 Phase 1 MVP

---

## 模块拆分

```
MODULE A: 链上类型定义 (crates/types/)         — 3h, 无依赖
MODULE B: 链上状态处理 (crates/state/)         — 8h, 依赖 A
MODULE C: 节点集成 (crates/node/)              — 5h, 依赖 B
MODULE D: claw-miner CLI (Python, 新项目)      — 10h, 可与 A-C 并行
MODULE E: 文档与分发                            — 3h, 依赖 D
```

## 并行执行策略

```
Agent 1 (Rust):  [A: Types 3h] → [B: State 8h] → [C: Chain 5h]
Agent 2 (Python): [D: CLI 8h] ─────────────────→ [D: 集成测试 2h]
Agent 3 (Docs):  ───────────────────────────────→ [E: Docs 3h]

总日历时间: ~16h（受 Rust 关键路径限制）
```

---

## MODULE A: 链上类型 (3h)

### A1. 新增 TxType

- [ ] **测试先行**: `test_tx_type_discriminant_values` — 断言 MinerRegister==15, MinerHeartbeat==16
- [ ] **测试先行**: `test_miner_register_payload_roundtrip` — borsh 序列化往返
- [ ] **测试先行**: `test_miner_heartbeat_payload_roundtrip`
- [ ] **实现**: `crates/types/src/transaction.rs` 添加 `MinerRegister = 15`, `MinerHeartbeat = 16`
- [ ] **实现**: 添加 `MinerRegisterPayload { tier: u8, ip_addr: Vec<u8>, name: String }`
- [ ] **实现**: 添加 `MinerHeartbeatPayload { latest_block_hash: [u8;32], latest_height: u64 }`

### A2. 新增状态类型

- [ ] **测试先行**: `test_miner_info_roundtrip` — borsh 序列化往返
- [ ] **测试先行**: `test_miner_tier_roundtrip`
- [ ] **实现**: `crates/types/src/state.rs` 添加 `MinerTier`, `MinerInfo` 结构体
- [ ] **实现**: 添加常量 `MINER_HEARTBEAT_INTERVAL`, `MAX_MINERS_PER_SUBNET`, `MINER_GRACE_BLOCKS`, 信誉阶梯常量

### A3. 验收

- [ ] `cargo test -p claw-types` 全部通过
- [ ] 无编译 warning

---

## MODULE B: 链上状态处理 (8h)

### B1. 错误类型

- [ ] **实现**: `crates/state/src/error.rs` 添加 `MinerAlreadyRegistered`, `MinerNotRegistered`, `MinerNameTooLong`, `SubnetLimitReached`, `HeartbeatTooEarly`, `InvalidMinerTier`, `InvalidIpLength`

### B2. WorldState 扩展

- [ ] **实现**: `crates/state/src/world.rs` 添加 `miners: BTreeMap<[u8;32], MinerInfo>`
- [ ] **实现**: 添加 `miner_heartbeat_tracker: BTreeMap<([u8;32], u64), bool>`
- [ ] **实现**: `state_root()` 中添加 miners + heartbeat_tracker 的 Merkle 叶
- [ ] **实现**: `apply_tx()` 添加 MinerRegister / MinerHeartbeat 分发
- [ ] **实现**: MinerHeartbeat 交易免 gas（修改 gas 扣除逻辑）

### B3. Handler 函数

- [ ] **测试先行**: `test_miner_register_success`
- [ ] **测试先行**: `test_miner_register_duplicate`
- [ ] **测试先行**: `test_miner_register_invalid_tier`
- [ ] **测试先行**: `test_miner_register_subnet_limit`
- [ ] **测试先行**: `test_miner_register_invalid_ip`
- [ ] **测试先行**: `test_miner_heartbeat_success`
- [ ] **测试先行**: `test_miner_heartbeat_not_registered`
- [ ] **测试先行**: `test_miner_heartbeat_too_early`
- [ ] **测试先行**: `test_miner_heartbeat_gas_free`
- [ ] **测试先行**: `test_miner_heartbeat_updates_reputation`
- [ ] **实现**: `handle_miner_register()` — 验证 + 注册
- [ ] **实现**: `handle_miner_heartbeat()` — 验证 + 更新

### B4. 奖励系统修改

- [ ] **测试先行**: `test_reward_per_block_new_schedule` — 验证所有减半周期
- [ ] **测试先行**: `test_reward_per_block_upgrade_transition` — 升级前后切换
- [ ] **测试先行**: `test_distribute_mining_rewards_basic` — 2 矿工按比例分配
- [ ] **测试先行**: `test_distribute_mining_rewards_no_miners`
- [ ] **测试先行**: `test_distribute_mining_rewards_respects_reputation`
- [ ] **测试先行**: `test_validator_reward_reduced_after_upgrade` — 65% 而非 100%
- [ ] **测试先行**: `test_update_miner_activity_deactivates`
- [ ] **测试先行**: `test_state_root_includes_miners`
- [ ] **实现**: `reward_per_block()` 添加 `MINING_UPGRADE_HEIGHT` 门控 + 几何减半
- [ ] **实现**: `distribute_block_reward()` 修改为升级后只分 65%
- [ ] **实现**: 新增 `distribute_mining_rewards()` — 35% 分给活跃矿工
- [ ] **实现**: 新增 `update_miner_activity()` — 超时未 heartbeat 标记不活跃

### B5. 验收

- [ ] `cargo test -p claw-state` 全部通过
- [ ] supply 完整性：mining 奖励 + validator 奖励 = base_reward（零和）
- [ ] state_root 确定性

---

## MODULE C: 节点集成 (5h)

### C1. 区块生产集成

- [ ] **实现**: `chain.rs produce_block()` 调用 `distribute_mining_rewards()`
- [ ] **实现**: `chain.rs apply_remote_block_inner()` 同样调用
- [ ] **实现**: epoch 边界调用 `update_miner_activity()`
- [ ] **实现**: supply 完整性检查适配新的双重奖励分发
- [ ] **验证**: produce_block 和 apply_remote_block 操作顺序一致

### C2. RPC 方法

- [ ] **实现**: `claw_getMinerInfo(address)` → MinerInfo | null
- [ ] **实现**: `claw_getMiners(active_only, limit, offset)` → MinerInfo[]
- [ ] **实现**: `claw_getMiningStats()` → 统计信息 JSON
- [ ] **实现**: `rpc_server.rs` 更新 `tx_type_name()`, `extract_to_and_amount()`

### C3. 查询方法

- [ ] **实现**: `chain.rs` 添加 `get_miner_info()`, `get_miners()`, `get_mining_stats()`

### C4. 验收

- [ ] `cargo test` 全部通过
- [ ] `cargo build --release` 无 warning
- [ ] 在 devnet 上手动测试：注册矿工 → 发 heartbeat → 收到挖矿奖励

---

## MODULE D: claw-miner CLI — Python 新项目 (10h)

### D1. 项目脚手架

- [ ] 创建 `claw-miner/` 目录结构
- [ ] `pyproject.toml` 配置（依赖: click, httpx, pynacl, blake3, rich, tomli）
- [ ] `src/clawminer/__init__.py`

### D2. 钱包模块

- [ ] **测试先行**: `test_generate_keypair`
- [ ] **测试先行**: `test_save_load_wallet`
- [ ] **测试先行**: `test_address_hex`
- [ ] **实现**: `wallet.py` — 生成/保存/加载 Ed25519 密钥对

### D3. RPC 客户端

- [ ] **测试先行**: `test_rpc_call_format`
- [ ] **测试先行**: `test_error_handling`
- [ ] **实现**: `rpc.py` — JSON-RPC 2.0 客户端

### D4. 交易构造

- [ ] **测试先行**: `test_miner_register_tx_format` — borsh 字节匹配 Rust
- [ ] **测试先行**: `test_miner_heartbeat_tx_format`
- [ ] **测试先行**: `test_signable_bytes` — 与 claw-node 一致
- [ ] **测试先行**: `test_sign_verify`
- [ ] **实现**: `tx.py` — 交易构造 + Ed25519 签名

### D5. 配置管理

- [ ] **测试先行**: `test_default_config`
- [ ] **测试先行**: `test_save_load_config`
- [ ] **实现**: `config.py` — TOML 配置管理

### D6. 挖矿主循环

- [ ] **测试先行**: `test_heartbeat_interval` — mock RPC 验证定时
- [ ] **实现**: `miner.py` — 注册 + heartbeat 循环 + 优雅停止

### D7. CLI 入口

- [ ] **实现**: `cli.py` — click 命令: init, start, stop, status, balance
- [ ] **实现**: `constants.py` — 共享常量

### D8. 跨语言兼容性测试

- [ ] **CRITICAL**: Python 构造的交易在 Rust 节点上能解析
- [ ] 方法：Python 生成 tx hex → 提交到 devnet → 验证上链成功

### D9. 验收

- [ ] `pytest` 通过，覆盖率 ≥ 80%
- [ ] `pip install -e .` 可用
- [ ] `claw-miner init` 生成钱包和配置
- [ ] `claw-miner start` 注册并发送 heartbeat
- [ ] `claw-miner status` 显示矿工信息
- [ ] `claw-miner balance` 显示余额

---

## MODULE E: 文档与分发 (3h)

### E1. AI Agent 安装文档

- [ ] **实现**: `docs/install.md` — 结构化给 AI Agent 阅读
- [ ] **实现**: `docs/update.md` — 更新指引

### E2. README

- [ ] **实现**: `README.md` — 安装、使用、CLI 参考

### E3. PyPI 发布

- [ ] `python -m build` 生成 wheel
- [ ] `twine upload` 到 PyPI
- [ ] 验证 `pip install clawminer` 可用

### E4. 验收

- [ ] AI Agent 读 install.md 能完成安装
- [ ] `pip install clawminer` 从 PyPI 安装成功

---

## 关键风险

| 风险 | 严重度 | 缓解 |
|------|--------|------|
| Python/Rust Borsh 格式不匹配 | CRITICAL | 跨语言往返测试 |
| 排放升级破坏现有 Validator | HIGH | MINING_UPGRADE_HEIGHT 门控 + 4 节点同步升级 |
| Supply 完整性被双重分发破坏 | HIGH | 两者都从同一池扣，测试验证零和 |
| Ed25519 PyNaCl/dalek 不兼容 | MEDIUM | 跨签名验证测试 |

---

## 最终验收 Checklist

- [ ] MinerRegister 交易上链成功
- [ ] MinerHeartbeat 交易上链成功（免 gas）
- [ ] 出块奖励升级后 65/35 分配
- [ ] 几何减半在所有周期正确
- [ ] Subnet 限制生效（/24 最多 3 个）
- [ ] 信誉乘数正确递进（0.2→0.5→1.0）
- [ ] 不活跃矿工停止获得奖励
- [ ] RPC 方法返回正确数据
- [ ] claw-miner CLI 全流程可用
- [ ] PyPI 包可安装
- [ ] `cargo test` 全部通过
- [ ] `pytest` 覆盖率 ≥ 80%
- [ ] Supply 完整性检查通过
- [ ] State root 在所有节点一致
