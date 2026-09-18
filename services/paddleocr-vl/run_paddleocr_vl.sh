#!/usr/bin/env bash
# 启动 PaddleOCR-VL 解析服务（Docker, HTTP API :50054）。
# 与 docreader / mineru 平级独立；raisfast 引擎 `paddleocr_vl` 调
# POST {endpoint}/layout-parsing（PaddleX serving 契约）。
set -euo pipefail
DIR="$(cd "$(dirname "$0")" && pwd)"
cd "$DIR"

PORT="${PADDLE_PORT:-50054}"
CONTAINER=raisfast-paddleocr-vl

if ! docker info >/dev/null 2>&1; then
  echo ">> docker 不可用，请先启动 Docker Desktop" >&2
  exit 1
fi

# 镜像本地构建（CPU 版；官方 vLLM 加速镜像为 NVIDIA GPU 专属，见 Dockerfile）
IMAGE="${PADDLE_IMAGE:-raisfast-paddleocr-vl:latest}"
if [ -z "$(docker images -q "$IMAGE" 2>/dev/null)" ]; then
  echo ">> 本地构建镜像 $IMAGE（pip 安装 paddlepaddle/paddleocr，首次较慢）"
  docker build -t "$IMAGE" "$DIR"
fi

if [ -z "$(docker ps -q -f name=^${CONTAINER}$)" ]; then
  if [ -n "$(docker ps -aq -f name=^${CONTAINER}$)" ]; then
    docker rm -f "${CONTAINER}" >/dev/null
  fi
  echo ">> PaddleOCR-VL serving on :${PORT}（模型首次请求时下载，走 Baidu 镜像源）"
  docker run -d --name "${CONTAINER}" \
    -p "${PORT}:8080" \
    -v raisfast-paddlex-models:/root/.paddlex \
    "$IMAGE" paddlex --serve --pipeline PaddleOCR-VL --host 0.0.0.0 --port 8080
fi

echo ">> 等待服务就绪…"
for _ in $(seq 1 120); do
  if curl -sf "http://localhost:${PORT}/docs" >/dev/null 2>&1; then
    echo ">> PaddleOCR-VL API 就绪: http://localhost:${PORT}（Swagger: /docs）"
    echo ">> raisfast 侧环境变量: RAISFAST_KB_PADDLEOCR_VL_ENDPOINT=http://localhost:${PORT}"
    exit 0
  fi
  sleep 3
done
echo ">> 超时未就绪，查看日志: docker logs ${CONTAINER}" >&2
exit 1
