# PaddleOCR-VL CPU 服务镜像（本地构建，Apple Silicon / x86 通用）。
# 官方加速镜像（paddleocr-genai-vllm-server）是 NVIDIA GPU 专属；
# CPU 场景改为：python 基础镜像 + 清华 PyPI 源安装 paddleocr/paddlex，
# 用 paddlex --serve 暴露 /layout-parsing（与 raisfast paddleocr_vl 引擎契约一致）。
FROM python:3.12-slim

ENV PIP_INDEX_URL=https://pypi.tuna.tsinghua.edu.cn/simple \
    PIP_DISABLE_PIP_VERSION_CHECK=1 \
    PADDLE_PDX_MODEL_SOURCE=BOS

# debian 源换清华镜像（默认源在本网络不可达）
RUN sed -i 's|deb.debian.org|mirrors.tuna.tsinghua.edu.cn|g' /etc/apt/sources.list.d/debian.sources \
    && apt-get update && apt-get install -y --no-install-recommends \
        libglib2.0-0 libgl1 curl \
    && rm -rf /var/lib/apt/lists/*

# paddlepaddle（CPU）+ paddleocr + paddlex serving
# paddlex 需要 [serving,ocr] extras（PaddleOCR-VL 管线依赖）
RUN pip install --no-cache-dir -U pip \
    && pip install --no-cache-dir "paddlepaddle==3.0.0" "paddleocr>=3.3" "paddlex[serving,ocr]"

EXPOSE 8080
CMD ["paddlex", "--serve", "--pipeline", "PaddleOCR-VL", "--host", "0.0.0.0", "--port", "8080"]
