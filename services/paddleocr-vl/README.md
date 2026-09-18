# PaddleOCR-VL 解析服务（独立引擎）

百度 PaddleOCR-VL 视觉语言模型解析管线，独立 Docker 容器，
与 `services/docreader`、`services/mineru` **平级、互不依赖**。

## 定位（与 MinerU 的分工）

- **扫描件/图片密集文档**（发票、手写、低质量扫描）→ PaddleOCR-VL OCR 更强
- **复杂版面学术文档**（公式/双栏/跨页表格）→ MinerU 更强
- KB 解析规则可按文件类型把两者混用

## 启动

**macOS（推荐，原生不经 Docker）**——官方加速镜像仅 NVIDIA GPU，且
Apple Silicon Docker 里 paddle arm64 轮子有 libpaddle 加载缺陷：

```bash
./run_paddleocr_vl_macos.sh
# 自动建 venv + 装 paddlepaddle/paddleocr/paddlex（清华源）
# 前台运行；模型首次启动下载（约数 GB）；Ctrl-C 停止
# 就绪后 Swagger: http://localhost:50054/docs
```

**Docker（Linux NVIDIA GPU 主机，生产路线）**：

```bash
docker run --gpus all ... \
  ccr-2vdh3abv-pub.cnc.bj.baidubce.com/paddlepaddle/paddleocr-genai-vllm-server:latest-nvidia-gpu ...
```

- 模型卷持久化；VL 模型推理建议内存 ≥ 8GB

## 与 raisfast 的对接

- 环境：`RAISFAST_KB_PADDLEOCR_VL_ENDPOINT=http://localhost:50054`
  （设置即注册 `paddleocr_vl` 引擎，重启生效）
- 路由：`parser_config` 规则 `{"file_types":["pdf","png","jpg"],"engine":"paddleocr_vl"}`
- 契约：`POST {endpoint}/layout-parsing`（base64 文件 + 识别参数，
  同步返回逐页 markdown + 内联 base64 图片）——与 `paddleocr_vl.rs` 一致，
  识别参数（跨页表格合并/标题重建/页眉页脚剥离）逐项对齐 WK
