# ClawNetwork Validator Economics & Onboarding Design

> Created: 2026-03-20. Living document — update as the network evolves.

## 1. Token Supply & Genesis Allocation

Total supply defined at genesis. Five allocation pools:

| Pool | Share | Purpose |
|------|-------|---------|
| Node Incentives | 40% | Block rewards, distributed over time |
| Ecosystem Fund | 25% | Grants, partnerships, ecosystem growth |
| Team | 15% | Core team, vesting schedule |
| Early Contributors | 10% | Advisors, early supporters |
| Liquidity Reserve | 10% | Market making, exchange listings |

Block rewards are deducted from the Node Incentives pool each block. When the pool is empty, no more block rewards are distributed (deflationary cap).

Transaction fees: 50% to block proposer, 20% to ecosystem fund, 30% burned.

## 2. Initial Validator Staking — How Token Comes From

### How other chains do it

| Chain | Initial Stake Source | Validator Cost |
|-------|---------------------|---------------|
| **Ethereum** | Validators buy 32 ETH on market | ~$80,000+ |
| **Cosmos** | ICO raised $17M → tokens distributed → validators self-stake | Varies |
| **Solana** | Foundation delegation + validators stake ~100 SOL themselves | ~$2,000 + delegation |
| **Polkadot** | Foundation "1000 Validators Programme" + validator self-stake | Small self-stake required |
| **Avalanche** | Foundation delegation to qualified validators | Technical requirements |
| **Aptos/Sui** | Testnet performance → mainnet delegation | Testnet participation |

**Common pattern**: All chains use some form of "Foundation Delegation" in early stages. Pure free tokens are never given — there's always skin in the game.

### ClawNetwork approach

**Phase 1 — Foundation Self-Operation (Current)**

ClawLabz operates all validators. Owner cold wallet delegates to 4 self-run nodes.

- Hetzner (Ashburn, VA)
- Aliyun (China)
- Mac Mini (Local)
- Win11 WSL (Local)

Owner Key: `71fa1a51...` (cold wallet, not on any server)
All block rewards flow to Owner Key automatically via delegated staking.

**Phase 2 — Invited Validators (Next)**

Trusted partners and community members can apply to run validators. ClawLabz delegates from Owner Key.

Admission requirements:
1. **Identity verification (KYC)** — passport/ID for individuals, business license for companies. One entity = one validator.
2. **Refundable security deposit** — small amount (e.g., 0.01 ETH / ~$25 USDT) to cover review costs and prove seriousness.
3. **Technical requirements** — independent IP, independent physical location, minimum specs: 4 vCPU / 8GB RAM / 100GB SSD.
4. **Probation period** — first month: 5,000 CLAW delegation. If uptime >95%, increased to 10,000 CLAW.
5. **Ongoing obligations** — 99.5% uptime target, monthly operations report. Foundation can revoke delegation at any time.

Anti-Sybil measures:
- KYC prevents multiple identities
- Unique IP + geolocation diversity required
- MAX_VALIDATORS = 21 caps total validator count
- Probation period catches fake operators before full delegation
- On-chain slashing (downtime >50% → 1% slash + jail 1 epoch)

**Phase 3 — Open Staking (Future)**

Once CLAW has market price (DEX or exchange listing):
- Anyone can buy CLAW and self-stake to become a validator (minimum: 10,000 CLAW)
- Token holders can delegate to validators and earn proportional rewards
- Validator competition: attract delegators by offering competitive commission rates
- Foundation retains some delegation power for ecosystem alignment

## 3. Sybil Attack Risk Analysis

### Attack scenario

Malicious actor creates N fake identities, passes KYC N times, runs N validator nodes.

| Validators controlled | Share of 21 | Can they attack consensus? | Reward share |
|----------------------|-------------|---------------------------|-------------|
| 1 | 4.8% | No | ~5% of rewards |
| 5 | 23.8% | No | ~24% of rewards |
| 10 | 47.6% | No (need >66%) | ~48% of rewards |
| 14+ | 66%+ | **YES — can double-spend** | ~67%+ |

### Mitigations

1. **MAX_VALIDATORS = 21**: Hard cap limits total slots
2. **Foundation retains 4+ validators**: Attacker needs 14 - 4 = 10 colluding validators for consensus attack
3. **KYC + unique IP + unique location**: Makes Sybil expensive (10 identities + 10 servers + 10 locations)
4. **Gradual onboarding**: Add 2-3 external validators at a time, not 10 at once
5. **Probation period**: Low initial delegation, increased only after proven track record
6. **On-chain slashing**: Automated penalty for misbehavior
7. **Foundation can undelegate**: Instant removal if suspicious activity detected

### Acceptable risk profile

With 4 foundation validators and gradual onboarding (2-3 external per quarter), the foundation maintains >50% of validation power for the first 1-2 years. This is consistent with how Solana, Cosmos, and Polkadot operated in their early stages.

Full decentralization is a gradual process, not a day-one requirement.

## 4. Reward Distribution

Block rewards are distributed proportionally by validator weight each block (~every 3-6 seconds).

With delegated staking:
- Rewards go directly to the **delegation owner** (cold wallet), not the validator server
- No transfer transaction — balance updated in state during block production
- On-chain auditability via `BlockEvent::RewardDistributed` events (v0.1.33+)
- Explorer shows rewards in block detail page

### Reward schedule

| Block range | Reward per block |
|-------------|-----------------|
| 0 - 999,999 | 10 CLAW |
| 1M - 1,999,999 | 8 CLAW |
| 2M - 2,999,999 | 6 CLAW |
| 3M - 3,999,999 | 4 CLAW |
| 4M - 9,999,999 | 2 CLAW |
| 10M+ | 1 CLAW |

Rewards are capped at the Node Incentives pool balance. When the pool is empty, only transaction fees remain.

## 5. Why Not Sell CLAW for ETH/USDT Now?

1. **No cross-chain bridge** — would need to build or integrate (large engineering effort)
2. **No market price** — how to determine CLAW/ETH rate with no trading history?
3. **Legal risk** — selling tokens may constitute securities issuance in many jurisdictions
4. **Chicken-and-egg** — need network value before token has value, need token value for meaningful staking

The standard path: build useful products on the chain → organic demand for CLAW → DEX listing → market-determined price → open staking.

## 6. Roadmap Summary

```
Phase 1 (Now)     → Foundation runs all validators, Owner Key delegates
Phase 2 (Next)    → Invite 5-10 external validators, KYC + deposit + probation
Phase 3 (DEX)     → CLAW tradeable on DEX, open self-staking
Phase 4 (Mature)  → Full decentralization, foundation reduces to <50% weight
```

## 7. Validator Set Size: Why 21 and Future Plans

### Current: MAX_VALIDATORS = 21

This follows the EOS/BNB Chain DPoS model — fewer validators for higher performance.

### Industry Comparison

| Chain | Validators | Consensus | Block Time | Trade-off |
|-------|-----------|-----------|------------|-----------|
| **EOS** | **21** | DPoS | 0.5s | Max performance, low decentralization |
| **BNB Chain** | **21** | PoSA | 3s | Same as EOS |
| **TRON** | **27** | DPoS | 3s | Slightly better than 21 |
| **Cosmos Hub** | 175 | BFT-PoS | 6s | Balanced |
| **Polkadot** | 297 | NPoS | 6s | More decentralized |
| **Ethereum** | ~900,000 | PoS | 12s | Maximum decentralization |
| **Solana** | ~1,500 | PoH+PoS | 0.4s | High performance + many validators |
| **ClawNetwork** | **21** | BFT-PoS | 3s | Performance-first for AI Agent workloads |

### Why fewer validators = faster

BFT consensus requires >2/3 validators to sign each block. More validators = more network round trips = slower finality. With 21 validators, finality is nearly instant.

### Advantages of 21

- **Fast block production** (3s) — critical for AI Agent interactions
- **Low communication overhead** — O(n²) message complexity in BFT
- **Simpler operations** — fewer validators to coordinate
- **Quick finality** — important for ClawArena, ClawMarket real-time applications

### Disadvantages of 21

- **Low BFT threshold** — only 14 colluding validators needed for consensus attack
- **Centralization criticism** — EOS was widely criticized for "21 super nodes" being controlled by cartels
- **Limited access** — scarce slots make it hard for new validators to join
- **Censorship risk** — small number of validators easier for governments to pressure

### EOS Lessons Learned

EOS's 21 Block Producers (BPs) became dominated by a few interest groups who voted for each other, forming a cartel. Community criticism was intense. Takeaway: 21 works technically but requires strong governance to prevent capture.

### Evolution Plan

```
Now:        21 validators (sufficient — only 4 active nodes currently)
Mid-term:   51 validators (balance between performance and decentralization)
            Cosmos Hub started at 100, gradually increased to 175
Long-term:  101 validators (adjust based on network scale and demand)
```

The change is a single constant:
```rust
// crates/consensus/src/types.rs
pub const MAX_VALIDATORS: usize = 21; // → 51 or 101 later
```

No protocol change needed — just update the constant and deploy new binary.

## 8. Validator Deposit Model (Phase 2)

### Why not free delegation?

Free delegation creates Sybil attack risk — malicious actors create multiple identities to capture validator slots and rewards. Even with KYC, the cost of attack must be economically meaningful.

### Why not sell CLAW tokens?

1. No cross-chain bridge exists yet
2. No market price for CLAW — how to determine fair rate?
3. Legal risk — token sale may constitute securities issuance
4. Chicken-and-egg — need network value before token has value

### Recommended: Deposit + Delegation Model

Validators pay a USDT deposit to ClawLabz. In return, ClawLabz delegates CLAW to their node. The validator does NOT own the CLAW — it's a delegation, revocable by the foundation.

This is a **service agreement**, not a token sale. Legal risk is significantly lower.

### Industry Benchmarks

| Chain | Validator self-stake requirement | USD value at launch | Total validators |
|-------|-------------------------------|---------------------|-----------------|
| **Ethereum** | 32 ETH | ~$80,000+ | ~900,000 |
| **Polkadot** (1KV) | ~5,000 DOT | ~$35,000 | 297 |
| **Avalanche** | 2,000 AVAX | ~$50,000 | ~1,200 |
| **Cosmos Hub** | Competitive self-stake | ~$10,000+ | 175 |
| **Solana** (Foundation) | 100 SOL self-stake | ~$15,000 | ~1,500 |

Note: Most chains' tokens already had market prices before mainnet (via ICO/private sale). ClawNetwork has no pre-sale, so deposit model is more appropriate at this stage.

### Tiered Deposit Structure

| Tier | Deposit | CLAW Delegation | Target |
|------|---------|----------------|--------|
| **Pioneer** | $2,000 USDT | 10,000 CLAW | Individual developers, community contributors |
| **Partner** | $5,000 USDT | 30,000 CLAW | Small companies, technical teams |
| **Enterprise** | $10,000 USDT | 100,000 CLAW | Institutions, professional validator operators |

### Why this range is reasonable

- $2,000 minimum: Sybil-ing 10 nodes costs $20,000 + $5,000/yr servers = $25,000. Sufficient deterrent.
- $10,000 maximum: Lower than Solana's ~$15,000, appropriate for an early-stage chain without token price.
- Tiered structure: different entry points for different participants. More deposit = more delegation = more rewards = fair.

### Deposit Refund Rules

| Condition | Refund |
|-----------|--------|
| 12 months operation + uptime >95% | 80% refund |
| 24 months operation + uptime >95% | 100% refund |
| Early exit or uptime <90% | No refund |
| Slashed (malicious behavior) | No refund + delegation revoked |

### Revenue Split

Block rewards for delegated validators:
- **80%** to validator operator (incentive to run well)
- **20%** to foundation (covers ecosystem costs)

Self-staked validators (Phase 3+): keep 100% of rewards minus on-chain commission.
