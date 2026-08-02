FROM rust:1-bookworm AS builder

WORKDIR /app

RUN apt-get update && apt-get install -y --no-install-recommends \
    protobuf-compiler \
    && rm -rf /var/lib/apt/lists/*

# Workspace + crate manifests (for dependency caching layer)
COPY Cargo.toml Cargo.lock ./
COPY crates/derive/ crates/derive/
COPY crates/core/Cargo.toml crates/core/Cargo.toml
COPY crates/core/build.rs crates/core/build.rs
COPY 3rd/bore/ 3rd/bore/
COPY migrations/ migrations/
RUN mkdir -p crates/core/src && echo "fn main() {}" > crates/core/src/main.rs
RUN cargo build --release --features "db-sqlite plugin-all search-tantivy storage-s3" || true
RUN rm -rf crates/core/src

# Real sources + embedded assets
COPY crates/core/src/ crates/core/src/
COPY crates/core/tests/ crates/core/tests/
COPY adminui/ adminui/
COPY templates/ templates/
COPY plugin-sdk/ plugin-sdk/
RUN touch crates/core/src/main.rs \
    && cargo build --release --features "db-sqlite plugin-all search-tantivy storage-s3"

FROM debian:bookworm-slim

RUN apt-get update && apt-get upgrade -y && apt-get install -y --no-install-recommends \
    ca-certificates \
    curl \
    && rm -rf /var/lib/apt/lists/*

RUN groupadd --gid 1000 app \
    && useradd --uid 1000 --gid app --shell /bin/bash --create-home app

WORKDIR /app

COPY --from=builder /app/target/release/raisfast /app/raisfast

RUN mkdir -p /app/data /app/logs /app/uploads /app/plugins-data \
    && chown -R app:app /app/data /app/logs /app/uploads /app/plugins-data

USER app

ENV APP_HOST=0.0.0.0
ENV APP_PORT=9898

EXPOSE 9898

HEALTHCHECK --interval=30s --timeout=5s --start-period=10s --retries=3 \
    CMD curl -f http://localhost:9898/healthz || exit 1

CMD ["./raisfast"]
