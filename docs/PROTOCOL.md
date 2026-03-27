# ClawNetwork Protocol Specification v0.1

## 1. Notation

- All multi-byte integers are **little-endian**.
- Byte arrays are fixed-length unless noted as `Vec<u8>`.
- Serialization uses **borsh** (Binary Object Representation Serializer for Hashing).
- Hash function: **blake3** (256-bit / 32 bytes).
- Signature scheme: **Ed25519** (RFC 8032).

---

## 2. Addresses

An address is a 32-byte Ed25519 public key.

```
Address = [u8; 32]  // Ed25519 verifying key bytes
```

The native token (CLW) uses the **zero address** `[0u8; 32]` as its token ID.

---

## 3. Transaction Format

### 3.1 Transaction Envelope

| Field | Type | Description |
|-------|------|-------------|
| `tx_type` | `u8` | Transaction type discriminator (0–5) |
| `from` | `[u8; 32]` | Sender address |
| `nonce` | `u64` | Replay protection counter |
| `payload` | `Vec<u8>` | Borsh-encoded type-specific data |
| `signature` | `[u8; 64]` | Ed25519 signature |

### 3.2 Signable Bytes

The signed message is the concatenation of all fields **except** `signature`:

```
signable = tx_type (1 byte) || from (32 bytes) || nonce (8 bytes LE) || payload (variable)
```

### 3.3 Transaction Hash

```
tx_hash = blake3(borsh_serialize(full_transaction))
```

### 3.4 Nonce Semantics

- Each address maintains a **nonce counter**, starting at 0.
- A transaction is valid only if `tx.nonce == current_nonce + 1`.
- On successful execution, the sender's nonce increments to `tx.nonce`.
- Nonce prevents replay attacks and establishes transaction ordering per sender.

---

## 4. Transaction Types

### 4.0 Type Discriminants

| Value | Name | Description |
|-------|------|-------------|
| 0 | `AgentRegister` | Register agent identity |
| 1 | `TokenTransfer` | Transfer native CLW |
| 2 | `TokenCreate` | Create custom token |
| 3 | `TokenMintTransfer` | Transfer custom token |
| 4 | `ReputationAttest` | Write reputation record |
| 5 | `ServiceRegister` | Register/update service |

### 4.1 AgentRegister (0)

**Payload:**

| Field | Type | Description |
|-------|------|-------------|
| `name` | `String` | Human-readable agent name |
| `metadata` | `BTreeMap<String, String>` | Arbitrary key-value pairs |

**Rules:**
- Sender address must not already be registered.
- `name` must be 1–64 bytes UTF-8.
- `metadata` may contain up to 16 entries, each key ≤ 32 bytes, each value ≤ 256 bytes.

### 4.2 TokenTransfer (1)

**Payload:**

| Field | Type | Description |
|-------|------|-------------|
| `to` | `[u8; 32]` | Recipient address |
| `amount` | `u128` | Amount in base units |

**Rules:**
- `amount` must be > 0.
- Sender must have sufficient CLW balance (including gas fee).
- `to` may be any valid address (does not need to be registered).

### 4.3 TokenCreate (2)

**Payload:**

| Field | Type | Description |
|-------|------|-------------|
| `name` | `String` | Token name (1–64 bytes) |
| `symbol` | `String` | Token symbol (1–8 bytes, uppercase) |
| `decimals` | `u8` | Decimal places (0–18) |
| `total_supply` | `u128` | Total supply in base units |

**Rules:**
- Sender must be a registered agent.
- Token ID is computed as: `blake3(sender_address || name || sender_nonce)`.
- Token ID must not already exist.
- `total_supply` must be > 0.
- Entire supply is credited to the issuer on creation.

### 4.4 TokenMintTransfer (3)

**Payload:**

| Field | Type | Description |
|-------|------|-------------|
| `token_id` | `[u8; 32]` | Custom token ID |
| `to` | `[u8; 32]` | Recipient address |
| `amount` | `u128` | Amount in base units |

**Rules:**
- `amount` must be > 0.
- Sender must have sufficient balance of the specified token.
- `token_id` must be a valid existing custom token (not `[0; 32]` — use TokenTransfer for CLW).

### 4.5 ReputationAttest (4)

**Payload:**

| Field | Type | Description |
|-------|------|-------------|
| `to` | `[u8; 32]` | Target agent address |
| `category` | `String` | Category (1–32 bytes, e.g., "game", "task") |
| `score` | `i16` | Score from -100 to +100 |
| `platform` | `String` | Source platform (1–64 bytes) |
| `memo` | `String` | Optional note (0–256 bytes) |

**Rules:**
- Both sender and `to` must be registered agents.
- Sender cannot attest to themselves (`from != to`).
- `score` must be in range [-100, +100].

### 4.6 ServiceRegister (5)

**Payload:**

| Field | Type | Description |
|-------|------|-------------|
| `service_type` | `String` | Service category (1–64 bytes) |
| `description` | `String` | Description (0–512 bytes) |
| `price_token` | `[u8; 32]` | Accepted payment token ID |
| `price_amount` | `u128` | Price per unit |
| `endpoint` | `String` | Service URL (1–256 bytes) |
| `active` | `bool` | Whether service is active |

**Rules:**
- Sender must be a registered agent.
- If sender already has a service of the same `service_type`, it is updated (upsert).
- Setting `active = false` effectively deregisters the service.

---

## 5. Gas

- **Fee**: 0.001 CLW per transaction = 1,000,000 base units (at 9 decimals).
- Gas is deducted from the sender **before** payload execution.
- If sender has insufficient balance for gas, the transaction is rejected (not included in block).
- Gas fees are **burned** (removed from circulation permanently).

---

## 6. Block Format

| Field | Type | Description |
|-------|------|-------------|
| `height` | `u64` | Block number (0 = genesis) |
| `prev_hash` | `[u8; 32]` | Previous block hash (zeros for genesis) |
| `timestamp` | `u64` | Unix timestamp (seconds) |
| `validator` | `[u8; 32]` | Block producer address |
| `transactions` | `Vec<Transaction>` | Ordered transactions |
| `state_root` | `[u8; 32]` | Merkle root of world state |
| `hash` | `[u8; 32]` | Block hash |

### 6.1 Block Hash

```
block_hash = blake3(
    height (8 bytes LE) ||
    prev_hash (32 bytes) ||
    timestamp (8 bytes LE) ||
    validator (32 bytes) ||
    for each tx: tx_hash (32 bytes) ||
    state_root (32 bytes)
)
```

### 6.2 Block Time

- Target: **3 seconds** per block.
- If no transactions are pending and no consensus timeout, empty blocks may be skipped.

---

## 7. World State

The world state is a collection of key-value maps:

| Map | Key | Value |
|-----|-----|-------|
| `balances` | `Address` | `u128` (CLW balance) |
| `token_balances` | `(Address, TokenId)` | `u128` |
| `nonces` | `Address` | `u64` |
| `agents` | `Address` | `AgentIdentity` |
| `tokens` | `TokenId` | `TokenDef` |
| `reputation` | append-only list | `ReputationAttestation` |
| `services` | `(Address, ServiceType)` | `ServiceEntry` |

### 7.1 State Root

The state root is a Merkle tree computed over all state entries:

1. Each state entry is serialized to bytes: `key_bytes || value_bytes`.
2. Each entry is hashed: `leaf = blake3(entry_bytes)`.
3. Leaves are sorted lexicographically.
4. Binary Merkle tree is computed with blake3 internal nodes.

---

## 8. Genesis Block

The genesis block (height 0) has:
- `prev_hash`: all zeros
- `validator`: all zeros
- `transactions`: empty
- Pre-configured initial state:
  - CLW balances per tokenomics allocation
  - No registered agents
  - No custom tokens
  - No reputation records
  - No services

### 8.1 Initial Token Distribution

| Allocation | Percentage | Amount (base units, 9 decimals) |
|------------|------------|--------------------------------|
| Node Incentives Pool | 40% | 400,000,000 × 10^9 |
| Ecosystem Fund | 25% | 250,000,000 × 10^9 |
| Team (locked) | 15% | 150,000,000 × 10^9 |
| Early Contributors | 10% | 100,000,000 × 10^9 |
| Liquidity Reserve | 10% | 100,000,000 × 10^9 |

---

## 9. P2P Messages

### 9.1 Gossip Messages (fire-and-forget)

| Message | Payload | Description |
|---------|---------|-------------|
| `TxBroadcast` | `Transaction` | New transaction to propagate |
| `BlockAnnounce` | `Block` (header only) | New block notification |

### 9.2 Request-Response

| Request | Response | Description |
|---------|----------|-------------|
| `GetBlocks { from: u64, count: u32 }` | `Vec<Block>` | Fetch blocks for sync |
| `GetStateSnapshot { height: u64 }` | `WorldState` | Full state at height |
| `GetPeers` | `Vec<PeerInfo>` | Peer discovery |

### 9.3 Consensus Messages

| Message | Payload | Description |
|---------|---------|-------------|
| `BlockProposal` | `Block` | Proposer broadcasts candidate block |
| `Vote` | `{ block_hash, height, voter, signature }` | Validator vote |

---

## 10. Consensus: PoS + Agent Score

### 10.1 Validator Set

- Validators must stake a minimum CLW amount (TBD before mainnet).
- Active validator set: top 21 candidates by weight, recalculated every **epoch** (100 blocks).

### 10.2 Weight Calculation

```
weight = normalize(stake) × stake_ratio + normalize(agent_score) × score_ratio
```

- `normalize(x)`: value / sum_of_all_values in the candidate set.
- `agent_score`: aggregated reputation score from on-chain attestations.
- Cold start ratios: `stake_ratio = 0.7, score_ratio = 0.3`.
- Target ratios: `stake_ratio = 0.4, score_ratio = 0.6`.

### 10.3 Block Production

1. Each round (3s), a proposer is selected via `VRF(prev_block_hash || height)` weighted by validator weights.
2. Proposer collects transactions from mempool, applies them, builds a block.
3. Proposer broadcasts `BlockProposal`.
4. Other validators verify the block and broadcast `Vote`.
5. If ≥ 2/3 votes received within timeout → block is finalized (single-block finality).
6. If timeout expires without 2/3 votes → round skipped, next proposer selected.

### 10.4 Finality

Single-block finality: once a block receives 2/3+ validator votes, it is **final and irreversible**. No forks, no reorganizations.

---

## 11. CLW Token Parameters

| Parameter | Value |
|-----------|-------|
| Name | Claw Network Token |
| Symbol | CLW |
| Decimals | 9 |
| Total Supply | 1,000,000,000 (1 billion) |
| Base Unit | 1 CLW = 10^9 base units |
| Gas Fee | 0.001 CLW = 1,000,000 base units |
| Gas Burn | 100% (deflationary) |
| Block Reward | From Node Incentives Pool, 10-year linear decrease |

---

## 12. RPC Interface

JSON-RPC 2.0 over HTTP (default port 9710) and WebSocket.

### 12.1 Methods

| Method | Params | Returns |
|--------|--------|---------|
| `claw_blockNumber` | — | `u64` |
| `claw_getBlockByNumber` | `height: u64` | `Block \| null` |
| `claw_getBalance` | `address: hex` | `u128` (string) |
| `claw_getTokenBalance` | `address: hex, token_id: hex` | `u128` (string) |
| `claw_getAgent` | `address: hex` | `AgentIdentity \| null` |
| `claw_getReputation` | `address: hex` | `Vec<ReputationAttestation>` |
| `claw_getServices` | `service_type?: string` | `Vec<ServiceEntry>` |
| `claw_getTransactionReceipt` | `tx_hash: hex` | `TxReceipt \| null` |
| `claw_sendTransaction` | `tx: hex (borsh bytes)` | `tx_hash: hex` |
| `claw_getNonce` | `address: hex` | `u64` |
| `claw_getTokenInfo` | `token_id: hex` | `TokenDef \| null` |

### 12.2 WebSocket Subscriptions

| Subscription | Data |
|-------------|------|
| `newBlock` | `Block` header |
| `newTransaction` | `Transaction` |

### 12.3 Response Format

```json
{
  "jsonrpc": "2.0",
  "id": 1,
  "result": { ... }
}
```

Errors follow JSON-RPC 2.0 error format with codes:
- `-32600`: Invalid request
- `-32601`: Method not found
- `-32602`: Invalid params
- `-32000`: Transaction rejected (with message)

---

## Appendix A: Constants

```rust
const CLW_DECIMALS: u8 = 9;
const CLW_TOTAL_SUPPLY: u128 = 1_000_000_000_000_000_000; // 10^9 * 10^9
const GAS_FEE: u128 = 1_000_000; // 0.001 CLW
const NATIVE_TOKEN_ID: [u8; 32] = [0u8; 32];
const BLOCK_TIME_SECS: u64 = 3;
const EPOCH_BLOCKS: u64 = 100;
const MAX_VALIDATORS: usize = 21;
```
