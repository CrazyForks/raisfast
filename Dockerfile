FROM rust:1.87-bookworm AS builder

WORKDIR /app

RUN apt-get update && apt-get install -y --no-install-recommends \
    protobuf-compiler \
    && rm -rf /var/lib/apt/lists/*

COPY Cargo.toml Cargo.lock ./
RUN mkdir src && echo "fn main() {}" > src/main.rs
RUN cargo build --release --features "db-sqlite plugin-all search-tantivy storage-s3" 2>/dev/null || true
RUN rm -rf src

COPY src/ src/
COPY migrations/ migrations/
COPY extensions/ extensions/
RUN touch src/main.rs \
    && cargo build --release --features "db-sqlite plugin-all search-tantivy storage-s3"

FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

COPY --from=builder /app/target/release/raisfast /app/raisfast
COPY migrations/ migrations/
COPY extensions/ extensions/

RUN mkdir -p /app/data /app/logs /app/uploads /app/plugins-data

ENV APP_HOST=0.0.0.0
ENV APP_PORT=9898

EXPOSE 9898

CMD ["./raisfast"]
