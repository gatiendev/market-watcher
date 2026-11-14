# syntax=docker/dockerfile:1

FROM rust:1-bookworm AS builder
WORKDIR /app

# Dependencies are rebuilt only when Cargo.toml/lock change.
COPY Cargo.toml Cargo.lock ./
RUN mkdir -p src && echo "fn main() {}" > src/main.rs \
    && cargo build --release --locked \
    && rm -rf src

COPY src ./src
RUN touch src/main.rs && cargo build --release --locked \
    && cp target/release/market-watcher /market-watcher

FROM debian:bookworm-slim
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates curl \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --create-home --uid 10001 app \
    && mkdir -p /data \
    && chown -R app:app /data

COPY --from=builder /market-watcher /usr/local/bin/market-watcher

ENV DATABASE_PATH=/data/market.db \
    PORT=8791 \
    REFRESH_SECS=300 \
    RUST_LOG=info

USER app
EXPOSE 8791

HEALTHCHECK --interval=30s --timeout=5s --start-period=45s --retries=3 \
    CMD curl -fsS http://127.0.0.1:8791/api/health || exit 1

CMD ["market-watcher"]
