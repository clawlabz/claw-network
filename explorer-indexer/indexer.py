"""ClawNetwork Explorer Indexer — scans blocks and writes transactions to PostgreSQL."""

import json
import logging
import sys
import time

import psycopg2
import psycopg2.extras
import requests

import config

logging.basicConfig(
    level=logging.INFO,
    format="%(asctime)s [%(levelname)s] %(message)s",
    stream=sys.stdout,
)
log = logging.getLogger("indexer")

# ── RPC helpers ──────────────────────────────────────────────────────────────

_rpc_id = 0


def rpc_call(method: str, params: list | None = None):
    """Send a JSON-RPC request to the node."""
    global _rpc_id
    _rpc_id += 1
    body = {"jsonrpc": "2.0", "id": _rpc_id, "method": method, "params": params or []}
    resp = requests.post(config.RPC_URL, json=body, timeout=10)
    resp.raise_for_status()
    data = resp.json()
    if "error" in data and data["error"]:
        raise RuntimeError(f"RPC error: {data['error']}")
    return data.get("result")


def get_chain_height() -> int:
    return rpc_call("claw_blockNumber")


def get_block(height: int) -> dict | None:
    return rpc_call("claw_getBlockByNumber", [height])


def get_tx_by_hash(tx_hash: str) -> dict | None:
    return rpc_call("claw_getTransactionByHash", [tx_hash])


def get_tx_receipt(tx_hash: str) -> dict | None:
    try:
        return rpc_call("claw_getTransactionReceipt", [tx_hash])
    except Exception:
        return None


# ── DB helpers ───────────────────────────────────────────────────────────────


def connect_db():
    return psycopg2.connect(
        host=config.DB_HOST,
        port=config.DB_PORT,
        dbname=config.DB_NAME,
        user=config.DB_USER,
        password=config.DB_PASS,
    )


def get_last_height(cur) -> int:
    cur.execute(
        "SELECT last_height FROM explorer_sync_state WHERE network = %s",
        (config.NETWORK,),
    )
    row = cur.fetchone()
    return row[0] if row else 0


def update_sync_state(cur, height: int, status: str = "syncing", error_msg: str | None = None):
    cur.execute(
        """UPDATE explorer_sync_state
           SET last_height = %s, last_updated = NOW(), status = %s, error_message = %s
           WHERE network = %s""",
        (height, status, error_msg, config.NETWORK),
    )


def ensure_network_stats_table(cur):
    """Create explorer_network_stats table if it doesn't exist and initialize for this network."""
    cur.execute("""
        CREATE TABLE IF NOT EXISTS explorer_network_stats (
            id SERIAL PRIMARY KEY,
            network VARCHAR(50) NOT NULL UNIQUE,
            total_transactions BIGINT DEFAULT 0,
            total_addresses BIGINT DEFAULT 0,
            total_transfer_volume TEXT DEFAULT '0',
            last_indexed_height BIGINT DEFAULT 0,
            updated_at TIMESTAMPTZ DEFAULT NOW()
        )
    """)
    cur.execute("""
        INSERT INTO explorer_network_stats (network)
        VALUES (%s)
        ON CONFLICT (network) DO NOTHING
    """, (config.NETWORK,))


def ensure_fee_columns(cur):
    """Add fee columns to explorer_daily_stats if they don't exist."""
    for col, typ in [("daily_total_fees", "TEXT DEFAULT '0'"), ("daily_avg_fee", "TEXT DEFAULT '0'")]:
        cur.execute(f"""
            ALTER TABLE explorer_daily_stats
            ADD COLUMN IF NOT EXISTS {col} {typ}
        """)


def ensure_validator_history_table(cur):
    """Create explorer_validator_history table if it doesn't exist."""
    cur.execute("""
        CREATE TABLE IF NOT EXISTS explorer_validator_history (
            id SERIAL PRIMARY KEY,
            network VARCHAR(50) NOT NULL,
            epoch BIGINT NOT NULL,
            validator_address TEXT NOT NULL,
            stake TEXT DEFAULT '0',
            weight DOUBLE PRECISION DEFAULT 0,
            agent_score DOUBLE PRECISION DEFAULT 0,
            recorded_at TIMESTAMPTZ DEFAULT NOW(),
            UNIQUE(network, epoch, validator_address)
        )
    """)


def refresh_daily_stats(cur):
    """Rebuild explorer_daily_stats from explorer_transactions."""
    cur.execute("DELETE FROM explorer_daily_stats WHERE network = %s", (config.NETWORK,))
    cur.execute("""
        INSERT INTO explorer_daily_stats
            (date, network, tx_count, transfer_volume, unique_senders, unique_receivers)
        SELECT
            DATE(TO_TIMESTAMP(timestamp)),
            network,
            COUNT(*),
            COALESCE(SUM(CASE WHEN tx_type = 1 AND amount IS NOT NULL
                         THEN amount::numeric ELSE 0 END)::TEXT, '0'),
            COUNT(DISTINCT from_addr),
            COUNT(DISTINCT to_addr) FILTER (WHERE to_addr IS NOT NULL)
        FROM explorer_transactions
        WHERE network = %s
        GROUP BY DATE(TO_TIMESTAMP(timestamp)), network
    """, (config.NETWORK,))
    # type_distribution: build per-day type counts
    cur.execute("""
        SELECT DATE(TO_TIMESTAMP(timestamp)) AS d, tx_type, COUNT(*) AS cnt
        FROM explorer_transactions
        WHERE network = %s
        GROUP BY d, tx_type
        ORDER BY d
    """, (config.NETWORK,))
    daily_types: dict[str, dict[str, int]] = {}
    for row in cur.fetchall():
        day_str = str(row[0])
        daily_types.setdefault(day_str, {})[str(row[1])] = row[2]
    for day_str, dist in daily_types.items():
        cur.execute(
            "UPDATE explorer_daily_stats SET type_distribution = %s WHERE date = %s AND network = %s",
            (json.dumps(dist), day_str, config.NETWORK),
        )
    # Update fee statistics per day
    cur.execute("""
        UPDATE explorer_daily_stats d SET
            daily_total_fees = sub.total_fees,
            daily_avg_fee = sub.avg_fee
        FROM (
            SELECT
                DATE(TO_TIMESTAMP(timestamp)) AS d,
                network,
                COALESCE(SUM(CASE WHEN fee IS NOT NULL THEN fee::numeric ELSE 0 END)::TEXT, '0') AS total_fees,
                COALESCE(AVG(CASE WHEN fee IS NOT NULL THEN fee::numeric ELSE 0 END)::TEXT, '0') AS avg_fee
            FROM explorer_transactions
            WHERE network = %s
            GROUP BY DATE(TO_TIMESTAMP(timestamp)), network
        ) sub
        WHERE d.date = sub.d AND d.network = sub.network
    """, (config.NETWORK,))


def refresh_network_stats(cur):
    """Update cumulative network statistics from explorer_transactions."""
    cur.execute("""
        UPDATE explorer_network_stats SET
            total_transactions = (
                SELECT COUNT(*) FROM explorer_transactions WHERE network = %s
            ),
            total_addresses = (
                SELECT COUNT(DISTINCT addr) FROM (
                    SELECT from_addr AS addr FROM explorer_transactions WHERE network = %s
                    UNION
                    SELECT to_addr AS addr FROM explorer_transactions WHERE network = %s AND to_addr IS NOT NULL
                ) t
            ),
            total_transfer_volume = COALESCE((
                SELECT SUM(amount::numeric)::TEXT
                FROM explorer_transactions
                WHERE network = %s AND tx_type = 1 AND amount IS NOT NULL
            ), '0'),
            last_indexed_height = (
                SELECT COALESCE(MAX(block_height), 0) FROM explorer_transactions WHERE network = %s
            ),
            updated_at = NOW()
        WHERE network = %s
    """, (config.NETWORK,) * 6)


def snapshot_validators(cur):
    """Record current validator state, keyed by epoch. Idempotent via ON CONFLICT."""
    try:
        health = requests.get(f"{config.RPC_URL}/health", timeout=5).json()
        epoch = health.get("epoch", 0)
        if epoch == 0:
            return

        validators = rpc_call("claw_getValidators")
        if not validators:
            return

        rows = []
        for v in validators:
            rows.append((
                config.NETWORK,
                epoch,
                v.get("address", ""),
                str(v.get("stake", "0")),
                v.get("weight", 0),
                v.get("agentScore", 0),
            ))

        if rows:
            psycopg2.extras.execute_batch(cur, """
                INSERT INTO explorer_validator_history
                    (network, epoch, validator_address, stake, weight, agent_score)
                VALUES (%s, %s, %s, %s, %s, %s)
                ON CONFLICT (network, epoch, validator_address) DO NOTHING
            """, rows)
            log.info("Validator snapshot: epoch %d, %d validators", epoch, len(rows))
    except Exception as e:
        log.warning("Validator snapshot failed: %s", e)


INSERT_TX = """
    INSERT INTO explorer_transactions
        (hash, network, tx_type, type_name, from_addr, to_addr, amount, fee,
         nonce, block_height, tx_index, timestamp, success, payload_json)
    VALUES (%s, %s, %s, %s, %s, %s, %s, %s, %s, %s, %s, %s, %s, %s)
    ON CONFLICT (hash, network) DO NOTHING
"""


def extract_tx_hash(tx_raw: dict) -> str | None:
    """Get the transaction hash from the block's transaction object."""
    h = tx_raw.get("hash")
    if not h:
        return None
    if isinstance(h, str):
        return h if h.startswith("0x") or len(h) == 64 else None
    # hash might be returned as a list of bytes
    if isinstance(h, list):
        return bytes(h).hex()
    return None


# ── Core indexing ────────────────────────────────────────────────────────────


def index_block(cur, height: int, fetch_receipt: bool = False):
    """Index all transactions in a single block."""
    block = get_block(height)
    if block is None:
        return 0

    txs_raw = block.get("transactions", [])
    if not txs_raw:
        return 0

    block_timestamp = block.get("timestamp", 0)
    rows = []

    for tx_idx, tx_raw in enumerate(txs_raw):
        tx_hash = extract_tx_hash(tx_raw)
        if not tx_hash:
            continue

        # Get parsed fields from node RPC
        tx_parsed = get_tx_by_hash(tx_hash)
        if not tx_parsed:
            continue

        tx_type = tx_parsed.get("txType", -1)

        # Determine success:
        # - Contract txs (6=Deploy, 7=Call, 18=UpgradeExecute): need receipt from node
        # - All other txs: if included in a block, they succeeded
        CONTRACT_TX_TYPES = {6, 7, 18}
        if tx_type in CONTRACT_TX_TYPES and fetch_receipt:
            receipt_data = get_tx_receipt(tx_hash)
            success = None
            if receipt_data:
                receipt = receipt_data.get("receipt", receipt_data)
                success = receipt.get("success")
        else:
            success = True if tx_type not in CONTRACT_TX_TYPES else None
        type_name = tx_parsed.get("typeName", f"Unknown({tx_type})")
        from_addr = tx_parsed.get("from", "")
        to_addr = tx_parsed.get("to")
        amount = tx_parsed.get("amount")
        fee = tx_parsed.get("fee", "1000000")
        nonce = tx_parsed.get("nonce", 0)
        timestamp = tx_parsed.get("timestamp", block_timestamp)

        # Store the complete RPC response as payload_json for auditability
        payload = tx_parsed

        rows.append((
            tx_hash,
            config.NETWORK,
            tx_type,
            type_name,
            from_addr,
            to_addr,
            amount,
            fee,
            nonce,
            height,
            tx_idx,
            timestamp,
            success,
            json.dumps(payload) if payload else None,
        ))

    if rows:
        psycopg2.extras.execute_batch(
            cur,
            INSERT_TX,
            rows,
        )

    return len(rows)


def run_backfill(conn, start_height: int, chain_height: int):
    """Backfill from start_height to chain_height in batches."""
    cur = conn.cursor()
    total_indexed = 0
    height = start_height + 1

    while height <= chain_height:
        batch_end = min(height + config.BATCH_SIZE, chain_height + 1)
        batch_count = 0

        for h in range(height, batch_end):
            batch_count += index_block(cur, h)

        update_sync_state(cur, batch_end - 1)
        conn.commit()

        total_indexed += batch_count
        progress = ((batch_end - 1) / chain_height) * 100 if chain_height > 0 else 100
        log.info(
            "Backfill %d-%d: %d txs indexed (%.1f%% complete, total: %d)",
            height, batch_end - 1, batch_count, progress, total_indexed,
        )

        height = batch_end

    return total_indexed


def run_realtime(conn, last_height: int):
    """Poll for new blocks and index them."""
    cur = conn.cursor()
    current = last_height
    blocks_since_refresh = 0
    blocks_since_daily_refresh = 0

    while True:
        try:
            chain_height = get_chain_height()

            if chain_height <= current:
                time.sleep(config.POLL_INTERVAL)
                continue

            for h in range(current + 1, chain_height + 1):
                count = index_block(cur, h, fetch_receipt=True)
                if count > 0:
                    log.info("Block %d: %d txs indexed", h, count)

            update_sync_state(cur, chain_height, status="idle")

            # Track blocks processed for periodic stats refresh
            blocks_since_refresh += chain_height - current
            blocks_since_daily_refresh += chain_height - current

            # Refresh network stats every 100 blocks
            if blocks_since_refresh >= 100:
                refresh_network_stats(cur)
                snapshot_validators(cur)
                blocks_since_refresh = 0

            # Refresh daily stats every 1000 blocks
            if blocks_since_daily_refresh >= 1000:
                refresh_daily_stats(cur)
                blocks_since_daily_refresh = 0

            conn.commit()
            current = chain_height

        except KeyboardInterrupt:
            raise
        except Exception as e:
            log.error("Realtime error: %s", e)
            conn.rollback()
            update_sync_state(cur, current, status="error", error_msg=str(e)[:500])
            conn.commit()
            time.sleep(config.ERROR_RETRY_INTERVAL)


# ── Main ─────────────────────────────────────────────────────────────────────


def main():
    log.info("Starting Explorer Indexer (network=%s, rpc=%s)", config.NETWORK, config.RPC_URL)

    if not config.DB_PASS:
        log.error("DB_PASS not set. Exiting.")
        sys.exit(1)

    conn = connect_db()
    log.info("Connected to database %s@%s:%d/%s", config.DB_USER, config.DB_HOST, config.DB_PORT, config.DB_NAME)

    cur = conn.cursor()
    ensure_network_stats_table(cur)
    ensure_fee_columns(cur)
    ensure_validator_history_table(cur)
    conn.commit()
    last_height = get_last_height(cur)
    chain_height = get_chain_height()
    log.info("Last indexed height: %d, chain height: %d", last_height, chain_height)

    # Backfill if behind
    if last_height < chain_height:
        update_sync_state(cur, last_height, status="syncing")
        conn.commit()

        log.info("Starting backfill from %d to %d (%d blocks)", last_height + 1, chain_height, chain_height - last_height)
        total = run_backfill(conn, last_height, chain_height)
        log.info("Backfill complete: %d transactions indexed", total)
        log.info("Refreshing daily stats...")
        refresh_daily_stats(cur)
        log.info("Daily stats refreshed")
        log.info("Refreshing network stats...")
        refresh_network_stats(cur)
        conn.commit()
        log.info("Network stats refreshed")
        last_height = chain_height

    # One-time fix: set success=true for non-contract txs that have null success
    cur.execute("""
        UPDATE explorer_transactions SET success = true
        WHERE success IS NULL AND tx_type NOT IN (6, 7, 18) AND network = %s
    """, (config.NETWORK,))
    fixed = cur.rowcount
    if fixed > 0:
        log.info("Fixed %d non-contract txs with null success → true", fixed)
        conn.commit()

    # Switch to realtime
    log.info("Entering realtime mode (poll every %ds)", config.POLL_INTERVAL)
    update_sync_state(cur, last_height, status="idle")
    conn.commit()

    run_realtime(conn, last_height)


if __name__ == "__main__":
    try:
        main()
    except KeyboardInterrupt:
        log.info("Shutting down.")
    except Exception as e:
        log.error("Fatal error: %s", e, exc_info=True)
        sys.exit(1)
