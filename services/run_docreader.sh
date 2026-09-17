#!/usr/bin/env bash
# 启动 vendored docreader 解析服务（gRPC :50051）。
# docreader 以自身为包运行，必须从本目录（services/）以模块方式启动，
# 并把本目录锚定进 PYTHONPATH（uv --project 会切换工作目录）。
set -euo pipefail
cd "$(dirname "$0")"

if [ ! -d docreader/.venv ]; then
  echo ">> first run: syncing docreader deps (use UV_DEFAULT_INDEX / --default-index for mirrors)"
  (cd docreader && uv sync --all-extras)
fi

echo ">> docreader gRPC on :${DOCREADER_GRPC_PORT:-50051}"
PYTHONPATH="$PWD" exec uv run --project docreader -m docreader.main
