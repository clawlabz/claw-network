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
    # type_distribution as separate update
    cur.execute("""
        UPDATE explorer_daily_stats ds SET type_distribution = sub.td
        FROM (
            SELECT DATE(TO_TIMESTAMP(timestamp)) AS d, network,
                   jsonb_object_agg(tx_type::TEXT, cnt) AS td
            FROM (
                SELECT timestamp, network, tx_type::TEXT, COUNT(*) AS cnt
                FROM explorer_transactions
                WHERE network = %s
                GROUP BY DATE(TO_TIMESTAMP(timestamp)), network, tx_type
            ) grouped
            GROUP BY d, network
        ) sub
        WHERE ds.date = sub.d AND ds.network = sub.network
    """, (config.NETWORK,))


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

        # Receipt: only fetch in realtime mode (historical receipts not persisted by node)
        success = None
        if fetch_receipt:
            receipt_data = get_tx_receipt(tx_hash)
            if receipt_data:
                receipt = receipt_data.get("receipt", receipt_data)
                success = receipt.get("success")

        tx_type = tx_parsed.get("txType", -1)
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
        conn.commit()
        log.info("Daily stats refreshed")
        last_height = chain_height

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
