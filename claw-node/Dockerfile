FROM rust:1.87-slim AS builder

WORKDIR /build
COPY Cargo.toml Cargo.lock ./
COPY crates/ crates/
RUN cargo build --release -p claw-node

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y ca-certificates && rm -rf /var/lib/apt/lists/*
COPY --from=builder /build/target/release/claw-node /usr/local/bin/claw-node

EXPOSE 9710 9711
VOLUME /data

HEALTHCHECK --interval=10s --timeout=3s --retries=3 \
  CMD curl -sf http://localhost:9710/health || exit 1

ENTRYPOINT ["claw-node"]
CMD ["start", "--data-dir", "/data", "--single"]
