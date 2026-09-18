#!/usr/bin/env bash
# 原生 macOS 启动 PaddleOCR-VL serving（不经 Docker）。
#
# 为什么有这个脚本：官方加速镜像（paddleocr-genai-vllm-server）仅支持
# NVIDIA GPU；Apple Silicon 的 Docker（linux/arm64）里 paddle 轮子存在
# libpaddle 加载缺陷；而 Paddle 官方提供 **macOS arm64 原生轮子**——
# 直接在宿主机 venv 里跑 paddlex --serve 是这台机器上最快的路线。
#
# raisfast 引擎 `paddleocr_vl` 调 POST {endpoint}/layout-parsing，
# 服务就绪后设置 RAISFAST_KB_PADDLEOCR_VL_ENDPOINT 即接入。
set -euo pipefail
DIR="$(cd "$(dirname "$0")" && pwd)"
cd "$DIR"

PORT="${PADDLE_PORT:-50054}"
VENV="$DIR/.venv"
PY="${PYTHON:-python3}"

if ! command -v "$PY" >/dev/null 2>&1; then
  echo ">> 未找到 python3，请先安装（brew install python@3.12）" >&2
  exit 1
fi

# 独立 venv，不污染系统 Python
if [ ! -x "$VENV/bin/python" ]; then
  echo ">> 创建虚拟环境 $VENV"
  "$PY" -m venv "$VENV"
fi

# 依赖：paddlepaddle（macOS arm64 原生轮子）+ paddleocr + paddlex[serving,ocr]
if ! "$VENV/bin/python" -c "import paddle, paddleocr, paddlex" 2>/dev/null; then
  echo ">> 安装依赖（清华 PyPI 源；paddlepaddle 数百 MB + 模型另计，耐心等）…"
  "$VENV/bin/pip" install --upgrade pip
  "$VENV/bin/pip" install -i https://pypi.tuna.tsinghua.edu.cn/simple \
    paddlepaddle "paddleocr>=3.3" "paddlex[serving,ocr]"
fi

echo ">> PaddleOCR-VL serving on :${PORT}（绑定 127.0.0.1）"
echo ">> 首次启动会下载 PaddleOCR-VL 模型（约数 GB，Baidu BOS 源）"
echo ">> raisfast 侧: RAISFAST_KB_PADDLEOCR_VL_ENDPOINT=http://localhost:${PORT}"
echo ">> 停止: Ctrl-C；就绪探针: http://localhost:${PORT}/docs"
exec "$VENV/bin/paddlex" --serve --pipeline PaddleOCR-VL --host 127.0.0.1 --port "$PORT"
