//! Prometheus metrics for the ClawNetwork node.

use prometheus::{
    Counter, Gauge, Histogram, HistogramOpts, Opts, Registry, TextEncoder, Encoder,
};
use std::sync::LazyLock;

/// Global metrics registry.
pub static REGISTRY: LazyLock<Registry> = LazyLock::new(Registry::new);

/// Total number of blocks produced/accepted.
pub static BLOCKS_TOTAL: LazyLock<Counter> = LazyLock::new(|| {
    let c = Counter::with_opts(Opts::new("blocks_total", "Total number of blocks")).unwrap();
    REGISTRY.register(Box::new(c.clone())).unwrap();
    c
});

/// Total number of transactions processed.
pub static TRANSACTIONS_TOTAL: LazyLock<Counter> = LazyLock::new(|| {
    let c = Counter::with_opts(Opts::new("transactions_total", "Total number of transactions processed")).unwrap();
    REGISTRY.register(Box::new(c.clone())).unwrap();
    c
});

/// Number of currently connected peers (validators in active set).
pub static PEERS_CONNECTED: LazyLock<Gauge> = LazyLock::new(|| {
    let g = Gauge::with_opts(Opts::new("peers_connected", "Number of connected peers")).unwrap();
    REGISTRY.register(Box::new(g.clone())).unwrap();
    g
});

/// Current mempool size (pending transactions).
pub static MEMPOOL_SIZE: LazyLock<Gauge> = LazyLock::new(|| {
    let g = Gauge::with_opts(Opts::new("mempool_size", "Number of pending transactions in mempool")).unwrap();
    REGISTRY.register(Box::new(g.clone())).unwrap();
    g
});

/// Current block height.
pub static BLOCK_HEIGHT: LazyLock<Gauge> = LazyLock::new(|| {
    let g = Gauge::with_opts(Opts::new("block_height", "Current block height")).unwrap();
    REGISTRY.register(Box::new(g.clone())).unwrap();
    g
});

/// Block production time in seconds.
pub static BLOCK_TIME_SECONDS: LazyLock<Histogram> = LazyLock::new(|| {
    let h = Histogram::with_opts(
        HistogramOpts::new("block_time_seconds", "Time between blocks in seconds")
            .buckets(vec![0.5, 1.0, 2.0, 3.0, 5.0, 10.0, 30.0, 60.0]),
    )
    .unwrap();
    REGISTRY.register(Box::new(h.clone())).unwrap();
    h
});

/// Encode all metrics as Prometheus text format.
pub fn gather() -> String {
    let encoder = TextEncoder::new();
    let metric_families = REGISTRY.gather();
    let mut buffer = Vec::new();
    encoder.encode(&metric_families, &mut buffer).unwrap();
    String::from_utf8(buffer).unwrap()
}
