# ============================================================
# RaisFast — fly.io production Dockerfile
# Usage:
#   1. Cross-compile locally:
#      cargo build --release --no-default-features \
#        --features "db-sqlite,plugin-js,plugin-rhai" \
#        --target x86_64-unknown-linux-musl
#
#   2. Deploy:
#      fly deploy --local-only
#
# The binary is built on YOUR machine. Only the compiled binary
# and static assets are sent to fly.io — zero source code exposure.
# ============================================================

FROM docker.1ms.run/library/debian:bookworm-slim

RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    curl \
    && rm -rf /var/lib/apt/lists/*

RUN groupadd --gid 1000 app \
    && useradd --uid 1000 --gid app --shell /bin/bash --create-home app

WORKDIR /app

COPY --chmod=755 target/x86_64-unknown-linux-musl/release/raisfast /app/raisfast
COPY adminui/ /app/adminui/

RUN mkdir -p /app/storage/db /app/storage/uploads /app/extensions/content_types /app/extensions/plugins \
    && chown -R app:app /app/storage /app/extensions

USER app

EXPOSE 9898

VOLUME ["/app/storage"]

CMD ["/app/raisfast"]
