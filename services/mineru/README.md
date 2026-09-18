# MinerU 解析服务（方案 A：独立引擎）

OpenDataLab MinerU 的 PDF→Markdown 解析服务，独立 Docker 容器，
与 `services/docreader` **平级、互不依赖**。

## 启动

```bash
./run_mineru.sh            # 首次会本地构建镜像（PyPI 清华源装 mineru[core]）
# 端口 50053（避让常用 8000；容器内仍是 8000，仅映射变化）
# 就绪后 Swagger: http://localhost:50053/docs
```

- 模型卷 `raisfast-mineru-models` 持久化（首次解析时经 ModelScope 下载）
- 镜像为本地构建（`Dockerfile`），Docker Hub 无 opendatalab/mineru 官方仓库
- Mac/容器内无 GPU → CPU 推理，大 PDF 每份需数分钟，属预期

## 与 raisfast 的对接

- 环境：`RAISFAST_KB_MINERU_URL=http://localhost:50053`（MineruEngine 实现时接入）
- 路由：`kb_knowledge_bases.parser_config` 规则 `{"file_types":["pdf"],"engine":"mineru"}`
  ——引擎名进解析注册表后，KB 级规则/全局默认即生效

## API 契约（待容器就绪后核对）

- 预期端点 `POST /file_parse`（multipart，`backend=pipeline`，
  `MINERU_MODEL_SOURCE=modelscope` 控制模型源）
- 响应含 markdown 与图片；Rust 侧 `MineruEngine` 需完成三件事：
  1. 图片落存储、改写 markdown 引用（对齐 docreader 的 `images/` 约定）
  2. 由 `pdf_info`/middle JSON 注入 `<!-- page:N -->` 锚点行（页映射/阅读视图依赖）
  3. 走 `parse_quality::gate` 质量门禁
