#!/usr/bin/env bash
# 启动 MinerU 解析服务（Docker, HTTP API :50053）。
# MinerU 与 docreader 互相独立：docreader 管 docx/ppt 等走原有链路，
# PDF 可按 KB 解析规则路由到 mineru（kb_knowledge_bases.parser_config）。
set -euo pipefail
DIR="$(cd "$(dirname "$0")" && pwd)"
cd "$DIR"

MINERU_PORT="${MINERU_PORT:-50053}"
MINERU_IMAGE="${MINERU_IMAGE:-raisfast-mineru:latest}"

# 镜像不存在则本地构建（PyPI 清华源 + ModelScope 模型，见 Dockerfile）
if [ -z "$(docker images -q "${MINERU_IMAGE}" 2>/dev/null)" ]; then
  echo ">> 本地构建镜像 ${MINERU_IMAGE}（首次需下载 mineru[core]，数 GB）"
  docker build -t "${MINERU_IMAGE}" "$DIR"
fi
CONTAINER=raisfast-mineru

if ! docker info >/dev/null 2>&1; then
  echo ">> docker 不可用，请先启动 Docker Desktop" >&2
  exit 1
fi

if [ -z "$(docker ps -q -f name=^${CONTAINER}$)" ]; then
  if [ -n "$(docker ps -aq -f name=^${CONTAINER}$)" ]; then
    docker rm -f "${CONTAINER}" >/dev/null
  fi
  echo ">> MinerU API on :${MINERU_PORT}（模型走 ModelScope 下载，首次解析较慢）"
  docker run -d --name "${CONTAINER}" \
    -p "${MINERU_PORT}:8000" \
    -e MINERU_MODEL_SOURCE=modelscope \
    -v raisfast-mineru-models:/root/.cache/mineru \
    "${MINERU_IMAGE}" mineru-api --host 0.0.0.0 --port 8000
fi

echo ">> 等待服务就绪（首次需下载模型，可能数分钟）…"
for _ in $(seq 1 120); do
  if curl -sf "http://localhost:${MINERU_PORT}/docs" >/dev/null 2>&1; then
    echo ">> MinerU API 就绪: http://localhost:${MINERU_PORT}（Swagger: /docs）"
    exit 0
  fi
  sleep 2
done
echo ">> 超时未就绪，查看日志: docker logs ${CONTAINER}" >&2
exit 1
