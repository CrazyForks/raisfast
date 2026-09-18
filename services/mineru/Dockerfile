# MinerU 解析服务镜像（本地构建）。
# 不用上游镜像名的原因：Docker Hub 无 opendatalab/mineru 官方仓库，
# 且本网络下拉取不便；改为 python 基础镜像 + PyPI 清华源安装 mineru[core]，
# 模型经 ModelScope 下载（MINERU_MODEL_SOURCE=modelscope）。
FROM python:3.12-slim

ENV MINERU_MODEL_SOURCE=modelscope \
    PIP_INDEX_URL=https://pypi.tuna.tsinghua.edu.cn/simple \
    PIP_DISABLE_PIP_VERSION_CHECK=1

# debian 源换清华镜像（默认源在本网络不可达）
RUN sed -i 's|deb.debian.org|mirrors.tuna.tsinghua.edu.cn|g' /etc/apt/sources.list.d/debian.sources \
    && apt-get update && apt-get install -y --no-install-recommends \
        libglib2.0-0 libgl1 curl \
    && rm -rf /var/lib/apt/lists/*

# mineru[core] 含 torch 等重依赖（数 GB，构建一次后续走缓存）
RUN pip install --no-cache-dir -U pip \
    && pip install --no-cache-dir "mineru[core]" fastapi "uvicorn[standard]"

EXPOSE 8000
CMD ["mineru-api", "--host", "0.0.0.0", "--port", "8000"]
