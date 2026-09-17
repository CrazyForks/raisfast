# docreader（文档解析服务，vendored）

WeKnora 的 docreader 文档解析服务，作为 raisfast KB 的**可选外部解析引擎**
（`kb-parser-engines-design.md` §2，引擎名 `docreader`，gRPC :50051）。

## 来源与许可

- 上游：<https://github.com/Tencent/WeKnora> 的 `docreader/` 目录
- 锁定 commit：`647848f3954dae34473b8a8d0e0eef5e0fb3a58e`（2026-09-07）
- 许可：MIT（见 `LICENSE.upstream`，Tencent/WeKnora 项目许可，含第三方组件例外条款）
- 同步方式：`rsync -a --exclude '__pycache__' ../third/WeKnora/docreader/ .`（third/ 为上游 checkout）

## 运行

> docreader 以自身为包运行（`from docreader.auth import …`），必须从**父目录以模块方式**启动
> [抄上游 Dockerfile.docreader: `uv run -m docreader.main`]。

```bash
# 本地源码跑——推荐脚本（首次自动 uv sync；镜像用 UV_DEFAULT_INDEX 环境变量）
./services/run_docreader.sh

# 等价手动形式（uv --project 会切工作目录，PYTHONPATH 必须锚定 services/）
cd services/
uv sync --project docreader --default-index https://pypi.tuna.tsinghua.edu.cn/simple
PYTHONPATH="$PWD" uv run --project docreader -m docreader.main

# 可选：挂 MinerU 后端增强扫描件（不配则用内置 pdfium 栅格化）
DOCREADER_MINERU_ENDPOINT=http://mineru:8000 uv run --project docreader -m docreader.main

# 官方镜像（不自管源码时的替代）
docker run -d -p 50051:50051 wechatopenai/weknora-docreader
```

raisfast 侧只需 `RAISFAST_KB_DOCREADER_URL=http://127.0.0.1:50051`；
KB 行 `parser_config.rules` 把 pdf/图片路由到 `docreader` 引擎。

注意：raisfast 的对接走 `ReadRequest.file_content`（字节进）+
`ImageRef.image_data`（字节回），**无需配置 MinIO/对象存储**。

## 环境变量（常用）

| 变量 | 默认 | 说明 |
|---|---|---|
| `DOCREADER_GRPC_PORT` / `PORT` | 50051 | gRPC 监听端口（注意：macOS 装了 Multipass 时 50051 被 `multipassd` 占用——用 50052 等替代，并同步 `RAISFAST_KB_DOCREADER_URL`） |
| `DOCREADER_MINERU_ENDPOINT` | — | MinerU 后端（扫描件增强） |
| `GRPC_AUTH_TOKEN` | — | gRPC 认证 token（内网可不开；设置后 raisfast 侧需带 metadata） |

完整配置见 `config.py`；上游文档见其 `README.md`。
