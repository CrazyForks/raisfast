# LLM API 网关技术参考

> 基于 one-api / new-api 的完整技术流程分析，作为 raisfast 实现 LLM Gateway 的参考文档。

## 1. 产品定位

LLM API 网关（也称"API 中转"或"API 聚合"）的核心价值：

- **统一入口**：客户端只需对接一个 API 地址，网关路由到不同上游
- **Key 管理**：统一颁发 API Key，控制额度、过期、模型权限
- **负载均衡**：多渠道分发，失败自动切换
- **格式转换**：OpenAI / Claude / Gemini 等格式互转
- **计费统计**：按 token 用量计费，支持缓存 token 折扣

## 2. 端到端请求流程

```
┌──────────┐     ┌──────────────────────────────────────────────────────┐     ┌──────────┐
│  Client   │────▶│                    raisfast Gateway                  │────▶│ Upstream  │
│           │◀────│                                                      │◀────│ (OpenAI/  │
│           │     │  RateLimit → Auth → Distribute → Relay → Response   │     │  Claude/  │
└──────────┘     └──────────────────────────────────────────────────────┘     │  Gemini)  │
                                                                                 └──────────┘
```

### 2.1 请求阶段

```
POST /v1/chat/completions
Authorization: Bearer sk-xxxxx

{
  "model": "gpt-4",
  "messages": [{"role":"user","content":"hello"}],
  "stream": true
}
```

### 2.2 中间件链

```
请求进入
  │
  ├─ Step 1: Rate Limit（IP 限流）
  │   · 滑动窗口，按 IP 计数
  │   · 默认 180 req / 3 min
  │   · Redis 或内存存储
  │
  ├─ Step 2: Token Auth（认证 + 预检）
  │   · 提取 Bearer token → sk-xxx
  │   · 查缓存 → DB 回退
  │   · 检查：enabled? expired? quota > 0?
  │   · IP 白名单验证
  │   · 模型白名单验证（token 级别）
  │   · 预扣额度（估算值，防止超额）
  │
  ├─ Step 3: Distribute（渠道选择）
  │   · 根据 user_group + model 查 abilities 表
  │   · 优先级 + 随机算法
  │   · 选中的渠道信息注入 context
  │
  ├─ Step 4: Relay（请求转发）
  │   · 格式转换（按上游类型）
  │   · 构建 HTTP 请求
  │   · 设置超时
  │   · 发送到上游
  │
  ├─ Step 5: Response（响应处理）
  │   · 非流式：解析 JSON → 提取 usage → 返回
  │   · 流式 SSE：逐行转发 → 累积 token → 结算
  │
  ├─ Step 6: Billing（计费结算）
  │   · 计算实际用量
  │   · 结算预扣额度
  │   · 记录日志
  │
  └─ 完成

失败重试（Step 4-5 之间）：
  · 429 / 5xx → 自动重试
  · 400 / 客户端错误 → 不重试
  · 每次重试换一个不同渠道
  · 超过重试次数 → 返回错误 + 退还额度
```

## 3. 数据库 Schema

### 3.1 核心表

#### channels — 上游渠道

```sql
CREATE TABLE channels (
    id              BIGINT PRIMARY KEY,
    document_id     VARCHAR(36) NOT NULL UNIQUE,
    tenant_id       VARCHAR(36),                    -- 多租户

    -- 渠道基本信息
    name            VARCHAR(255) NOT NULL,
    provider_type   SMALLINT NOT NULL DEFAULT 1,    -- 1=OpenAI, 3=Azure, 14=Anthropic, 15=Gemini ...
    status          SMALLINT NOT NULL DEFAULT 1,    -- 1=enabled, 2=manual_disabled, 3=auto_disabled
    base_url        VARCHAR(1024),                  -- 上游 API 地址
    api_key         TEXT NOT NULL,                  -- 上游 API Key（可存多个，逗号分隔）

    -- 路由配置
    models          TEXT NOT NULL,                  -- 支持的模型列表（逗号分隔）
    model_mapping   TEXT,                           -- {"客户端模型":"上游模型"} JSON
    priority        BIGINT NOT NULL DEFAULT 0,     -- 路由优先级，越高越优先
    weight          INT NOT NULL DEFAULT 0,         -- 权重（预留）
    channel_group   VARCHAR(255) DEFAULT 'default', -- 渠道分组

    -- 高级配置
    config          TEXT,                           -- 渠道特定配置 JSON
    system_prompt   TEXT,                           -- 强制注入的 system prompt
    header_override TEXT,                           -- 请求头覆盖 JSON
    param_override  TEXT,                           -- 请求参数覆盖 JSON
    status_code_mapping TEXT,                       -- 错误码映射 JSON

    -- 统计
    used_quota      BIGINT NOT NULL DEFAULT 0,
    response_time   INT,                            -- 平均响应时间 ms
    test_time       TIMESTAMPTZ,

    -- 审计
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
```

#### tokens — API Key 管理

```sql
CREATE TABLE tokens (
    id              BIGINT PRIMARY KEY,
    document_id     VARCHAR(36) NOT NULL UNIQUE,
    tenant_id       VARCHAR(36),
    user_id         BIGINT NOT NULL REFERENCES users(id),

    -- Key 信息
    name            VARCHAR(255) NOT NULL,
    key             VARCHAR(64) NOT NULL UNIQUE,    -- sk-xxxx
    status          SMALLINT NOT NULL DEFAULT 1,    -- 1=enabled, 2=disabled, 3=expired, 4=exhausted

    -- 额度控制
    remain_quota    BIGINT NOT NULL DEFAULT 0,
    used_quota      BIGINT NOT NULL DEFAULT 0,
    unlimited_quota BOOLEAN NOT NULL DEFAULT FALSE,

    -- 访问控制
    expired_at      TIMESTAMPTZ,                    -- 过期时间
    allowed_models  TEXT,                           -- 允许的模型列表（空=全部）
    allowed_ips     TEXT,                           -- IP 白名单 CIDR
    token_group     VARCHAR(255),                   -- Token 级分组覆盖

    -- 审计
    accessed_at     TIMESTAMPTZ,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
```

#### abilities — 路由交叉表

```sql
-- 预计算的 (group × model × channel) 关系
-- 避免 runtime 时复杂查询
CREATE TABLE abilities (
    channel_group   VARCHAR(255) NOT NULL,          -- 用户分组
    model           VARCHAR(255) NOT NULL,          -- 模型名
    channel_id      BIGINT NOT NULL REFERENCES channels(id),
    enabled         BOOLEAN NOT NULL DEFAULT TRUE,
    priority        BIGINT NOT NULL DEFAULT 0,
    PRIMARY KEY (channel_group, model, channel_id)
);

CREATE INDEX idx_abilities_group_model ON abilities(channel_group, model, enabled);
```

#### gateway_logs — 请求日志

```sql
CREATE TABLE gateway_logs (
    id              BIGINT PRIMARY KEY,
    tenant_id       VARCHAR(36),

    -- 请求信息
    request_id      VARCHAR(64),                    -- X-Request-ID
    user_id         BIGINT NOT NULL,
    token_id        BIGINT,
    channel_id      BIGINT,
    model_name      VARCHAR(255),
    is_stream       BOOLEAN NOT NULL DEFAULT FALSE,

    -- 用量
    prompt_tokens       INT NOT NULL DEFAULT 0,
    completion_tokens   INT NOT NULL DEFAULT 0,
    cache_tokens        INT NOT NULL DEFAULT 0,     -- 缓存命中 token

    -- 计费
    quota           BIGINT NOT NULL DEFAULT 0,      -- 本次消耗额度
    group_ratio     REAL,                            -- 分组倍率
    model_ratio     REAL,                            -- 模型倍率

    -- 性能
    elapsed_time    INT,                             -- 耗时 ms
    status_code     INT,                             -- 上游返回的 HTTP 状态码

    -- 审计
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_gateway_logs_user ON gateway_logs(user_id, created_at);
CREATE INDEX idx_gateway_logs_token ON gateway_logs(token_id, created_at);
CREATE INDEX idx_gateway_logs_channel ON gateway_logs(channel_id, created_at);
CREATE INDEX idx_gateway_logs_model ON gateway_logs(model_name, created_at);
```

#### pricing — 模型定价

```sql
CREATE TABLE pricing (
    id                  BIGINT PRIMARY KEY,
    document_id         VARCHAR(36) NOT NULL UNIQUE,
    tenant_id           VARCHAR(36),

    model_name          VARCHAR(255) NOT NULL,       -- 模型名

    -- 计费方式
    billing_mode        VARCHAR(20) NOT NULL DEFAULT 'ratio', -- ratio=倍率, price=固定价
    model_ratio         REAL NOT NULL DEFAULT 1.0,   -- 模型倍率
    completion_ratio    REAL NOT NULL DEFAULT 1.0,   -- 补全倍率
    cache_ratio         REAL,                         -- 缓存 token 倍率（默认 = model_ratio）
    model_price         REAL,                         -- 每次请求固定价格
    billing_expr        TEXT,                         -- 表达式计费（高级）

    created_at          TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at          TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
```

#### user_groups — 用户分组倍率

```sql
CREATE TABLE user_groups (
    id          BIGINT PRIMARY KEY,
    name        VARCHAR(255) NOT NULL UNIQUE,         -- default, vip, enterprise
    ratio       REAL NOT NULL DEFAULT 1.0,            -- 分组倍率
    created_at  TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
```

## 4. 渠道选择算法

### 4.1 基础算法（one-api）

```
输入: user_group, model
输出: channel

1. 查 abilities 表:
   WHERE channel_group = user_group
     AND model = model
     AND enabled = true

2. 取最高 priority 的记录集合

3. 在同 priority 集合内随机选一个

4. 返回对应 channel
```

### 4.2 增强算法（new-api）

```
输入: user_group, model, session_id
输出: channel

1. 会话亲和检查:
   · 如果 session_id 存在，查缓存看上次用的哪个 channel
   · 同一会话优先路由到同一 channel（保持上下文一致）

2. auto group 模式:
   · 如果 user_group = "auto"
   · 按优先级尝试多个 group
   · 第一个有可用 channel 的 group 生效

3. 多 Key 轮询:
   · 如果 channel 配置了多个 api_key
   · 按 round-robin 轮询使用

4. 降级:
   · 首次尝试: 最高 priority
   · 重试时: 忽略 priority，任何可用 channel
```

### 4.3 raisfast 建议的改进

```
1. 加权随机（weighted random）— 代替简单随机
   · channel.weight 字段实际生效
   · 响应时间越短 → 动态权重越高

2. 最少连接（least connections）
   · 跟踪每个 channel 的活跃连接数
   · 选连接数最少的

3. 健康检查
   · 后台定时 ping 上游
   · 异常自动降权或禁用
   · 恢复后自动提权

4. 地理路由（预留）
   · 根据 channel 的 region 和客户端位置
   · 优先路由到最近的 region
```

## 5. 格式转换

### 5.1 Adaptor 接口设计

```rust
/// LLM 提供商适配器接口
#[async_trait]
pub trait ProviderAdaptor: Send + Sync {
    /// 提供商名称
    fn provider_name(&self) -> &str;

    /// 将统一请求转换为上游格式
    fn convert_request(
        &self,
        unified: &UnifiedChatRequest,
        channel: &Channel,
    ) -> AppResult<ProviderRequest>;

    /// 构建上游 HTTP 请求
    fn build_http_request(
        &self,
        base_url: &str,
        api_key: &str,
        body: ProviderRequest,
    ) -> AppResult<reqwest::Request>;

    /// 处理非流式响应
    async fn handle_response(
        &self,
        resp: reqwest::Response,
    ) -> AppResult<UnifiedChatResponse>;

    /// 处理流式响应（返回 SSE 流）
    fn handle_stream_response(
        &self,
        resp: reqwest::Response,
    ) -> AppResult<Pin<Box<dyn Stream<Item = Result<SSEChunk>> + Send>>>;

    /// 从响应中提取 token 用量
    fn extract_usage(&self, response: &ProviderResponse) -> TokenUsage;

    /// 该提供商支持的模型列表
    fn supported_models(&self) -> &[&str];
}
```

### 5.2 统一请求结构

```rust
/// 统一的 Chat 请求（OpenAI 兼容格式作为内部标准）
pub struct UnifiedChatRequest {
    pub model: String,
    pub messages: Vec<ChatMessage>,
    pub temperature: Option<f64>,
    pub top_p: Option<f64>,
    pub max_tokens: Option<i64>,
    pub stream: bool,
    pub stop: Option<Vec<String>>,
    pub tools: Option<Vec<Tool>>,
    pub response_format: Option<ResponseFormat>,
}

pub struct ChatMessage {
    pub role: MessageRole,       // system, user, assistant, tool
    pub content: MessageContent, // text or array of content parts
    pub tool_calls: Option<Vec<ToolCall>>,
    pub tool_call_id: Option<String>,
}

pub struct TokenUsage {
    pub prompt_tokens: i64,
    pub completion_tokens: i64,
    pub cache_tokens: i64,       // 缓存命中的 token 数
    pub total_tokens: i64,
}
```

### 5.3 OpenAI → Claude 转换要点

```
OpenAI 格式                          Claude 格式
─────────────                        ──────────
messages: [                          system: "system prompt",  ← 提取到顶层
  {role:"system", content:"..."},    messages: [
  {role:"user", content:"hi"}           {role:"user", content:"hi"}
]                                    ]
max_tokens: 可选                     max_tokens: 必填（默认 4096）
stop: ["\n"]                         stop_sequences: ["\n"]    ← 字段名不同
tools: [{type:"function",...}]       tools: [{name:"",...}]    ← 格式不同
tool_choice: "auto"                  tool_choice: {type:"auto"}
stream: true                         stream: true
temperature: 0.7                     temperature: 0.7
```

### 5.4 OpenAI → Gemini 转换要点

```
OpenAI 格式                          Gemini 格式
─────────────                        ──────────
POST /v1/chat/completions            POST /v1/models/{model}:generateContent
{                                    {
  "model": "gpt-4",                   "contents": [
  "messages": [                            {"role":"user","parts":[{"text":"hi"}]}
    {role:"user", content:"hi"}       ],
  ],                                   "systemInstruction": {"parts":[{"text":"..."}]},
  "temperature": 0.7                   "generationConfig": {
}                                        "temperature": 0.7
                                       }
                                     }
```

### 5.5 透传优化（零拷贝）

```
当上游是 OpenAI 兼容 且 没有模型映射 且 没有 system prompt 注入时：
→ 直接转发原始请求体，不解析不序列化
→ 节省 CPU 和延迟
```

## 6. 流式转发（SSE）

### 6.1 架构

```
Upstream ──SSE──▶ raisfast ──SSE──▶ Client

                    ┌──────────────┐
                    │  SSE Proxy   │
                    │              │
                    │ line_scanner ◀─── 逐行读取上游
                    │      │       │
                    │      ▼       │
                    │  parse_chunk │─── 提取 usage
                    │      │       │
                    │      ▼       │
                    │  forward     │─── 逐行转发客户端
                    │      │       │
                    │      ▼       │
                    │  accumulate  │─── 累积 token 计数
                    └──────────────┘
```

### 6.2 SSE 协议

```
上游返回（chunked transfer encoding）:
data: {"id":"chatcmpl-xxx","choices":[{"delta":{"content":"Hello"},"index":0}]}

data: {"id":"chatcmpl-xxx","choices":[{"delta":{"content":" world"},"index":0}]}

data: {"id":"chatcmpl-xxx","choices":[{"delta":{},"index":0}],"usage":{"prompt_tokens":10,"completion_tokens":5}}

data: [DONE]
```

### 6.3 Token 计数策略

```
优先级：
1. 上游返回 usage 字段（最准确）
2. 强制 stream_options: {"include_usage": true} 让上游返回 usage
3. tiktoken 本地估算（退路）

流式场景的计费流程：
1. 请求开始：预扣额度 = max_tokens × model_ratio × group_ratio
2. 流式传输中：累积 completion_tokens
3. 流结束：结算 = (prompt_tokens + completion_tokens × completion_ratio) × group_ratio
4. 差额退还
```

### 6.4 Rust 实现（建议）

```rust
use axum::response::sse::{Event, Sse};
use futures::stream::Stream;
use reqwest::Response;

pub async fn proxy_sse(
    upstream_resp: Response,
) -> Sse<Pin<Box<dyn Stream<Item = Result<Event, axum::Error>> + Send>>> {
    let stream = async_stream::stream! {
        let mut lines = upstream_resp.lines();

        while let Some(line) = lines.next_line().await.unwrap_or(None) {
            if line.starts_with("data: ") {
                let data = &line[6..];

                if data == "[DONE]" {
                    yield Ok(Event::default().data("[DONE]"));
                    break;
                }

                // 提取 usage（如果有）
                if let Ok(chunk) = serde_json::from_str::<SSEChunk>(data) {
                    if let Some(usage) = chunk.usage {
                        // 记录 token 用量
                        record_usage(usage).await;
                    }
                }

                yield Ok(Event::default().data(data.to_string()));
            }
        }
    };

    Sse::new(Box::pin(stream))
}
```

## 7. 计费系统

### 7.1 额度计算公式

```
quota = group_ratio × model_ratio × (
    prompt_tokens
    + completion_tokens × completion_ratio
    - cache_tokens × (1 - cache_ratio)
)

其中:
- group_ratio:    用户分组倍率（VIP 打折）
- model_ratio:    模型倍率（GPT-4 比 GPT-3.5 贵）
- completion_ratio: 补全倍率（GPT-4 补全 = 2x prompt）
- cache_ratio:    缓存折扣（缓存 token 更便宜）
```

### 7.2 预扣 + 结算

```
时间线:
─────────────────────────────────────────────────────▶
   │                         │                    │
   │  预扣额度(estimate)      │  实际用量(usage)    │  结算(refund)
   │  remain_quota -= 预估值   │                    │  remain_quota += (预估 - 实际)
   │                         │                    │
   │  如果余额不足预扣 → 拒绝   │                    │  如果中途失败 → 全额退还
```

### 7.3 各模型倍率参考

```
模型                    model_ratio  completion_ratio
─────────────────────   ──────────   ───────────────
gpt-3.5-turbo           0.75         1.0
gpt-4                   15.0         2.0
gpt-4-turbo             5.0          2.0
gpt-4o                  2.5          2.0
claude-3-haiku          0.5          1.25
claude-3-sonnet         1.5          1.25
claude-3-opus           7.5          1.25
claude-3.5-sonnet       1.5          1.25
gemini-pro              1.0          1.0
gemini-1.5-pro          1.75         1.0
gemini-1.5-flash        0.075        1.0
deepseek-chat           0.14         1.0
deepseek-reasoner       0.55         1.0
```

## 8. 失败重试与容错

### 8.1 重试策略

```
RetryTimes = 可配置（默认 0）

重试条件:
  429 (Too Many Requests)     → 重试（上游限流）
  500 (Internal Server Error) → 重试
  502 (Bad Gateway)           → 重试
  503 (Service Unavailable)   → 重试
  504 (Gateway Timeout)       → 重试
  400 (Bad Request)           → 不重试（客户端问题）
  401 (Unauthorized)          → 不重试（Key 无效）
  403 (Forbidden)             → 不重试
  404 (Not Found)             → 不重试

每次重试:
  1. 选一个不同的 channel（排除上次失败的）
  2. 重新预扣额度
  3. 重新发送请求
```

### 8.2 渠道健康监控

```
每个 channel 维护一个成功率队列（最近 N 次请求）:

success_rate = successes / total_requests

如果 success_rate < threshold（默认 0.8）:
  → 自动禁用该 channel（status = auto_disabled）

后台定时检查:
  → 定期 ping 被禁用的 channel
  → 恢复后自动启用（status = enabled）
```

### 8.3 raisfast 改进建议

```
1. 熔断器（Circuit Breaker）:
   · Closed → Open → Half-Open → Closed
   · 比简单成功率监控更优雅

2. 指数退避重试:
   · retry 1: 100ms 后
   · retry 2: 200ms 后
   · retry 3: 400ms 后

3. 优先级感知:
   · 记录每个 channel 的平均延迟
   · 同优先级内优先选延迟低的

4. 渠道预热:
   · 新添加的 channel 先做健康检查
   · 确认可用后才加入路由池
```

## 9. Provider 类型常量

```
const OPENAI:            1
const AZURE:             3
const CUSTOM:            4   // 自定义 URL
const ANTHROPIC:        14
const GEMINI:           15
const BAIDU:            16
const ZHIPU:            17
const ALI:              18
const XUNFEI:           19
const AWS:              20
const COHERE:           21
const DEEPSEEK:         22
const MOONSHOT:         23
const BAICHUAN:         24
const MINIMAX:          25
const GROQ:             26
const OLLAMA:           27
const TONGYI_QWEN:      28  // 通义千问
const YI:               29  // 零一万物
const STEP:             30  // 阶跃星辰
const DOUBAO:           31  // 字节豆包
const COZE:             32
const CLOUDFLARE:       33
const DEEPL:            34
const TOGETHER_AI:      35
const DIFY:             36
const XAI:              37  // Grok
const SILICONFLOW:      38  // 硅基流动
const VERTEX_AI:        39  // Google Vertex AI
```

## 10. raisfast 实现建议

### 10.1 模块划分

```
src/
├── gateway/
│   ├── mod.rs              -- 模块入口
│   ├── router.rs           -- /v1/* 路由注册
│   ├── middleware/
│   │   ├── rate_limit.rs   -- IP 限流
│   │   ├── auth.rs         -- Token 认证
│   │   └── distribute.rs   -- 渠道选择
│   ├── relay/
│   │   ├── mod.rs          -- 转发入口 + 重试逻辑
│   │   ├── sse.rs          -- SSE 流式代理
│   │   ├── billing.rs      -- 预扣/结算
│   │   └── usage.rs        -- Token 计数
│   ├── adaptor/
│   │   ├── mod.rs          -- trait ProviderAdaptor
│   │   ├── openai.rs       -- OpenAI 透传
│   │   ├── anthropic.rs    -- Claude 格式转换
│   │   ├── gemini.rs       -- Gemini 格式转换
│   │   ├── azure.rs        -- Azure OpenAI
│   │   └── custom.rs       -- 自定义 OpenAI 兼容
│   ├── channel.rs          -- 渠道管理 (CRUD)
│   ├── token.rs            -- API Key 管理 (CRUD)
│   ├── ability.rs          -- 路由交叉表维护
│   ├── pricing.rs          -- 模型定价
│   └── health.rs           -- 渠道健康检查
├── models/
│   ├── channel.rs          -- Channel model
│   ├── gateway_token.rs    -- GatewayToken model
│   ├── gateway_log.rs      -- GatewayLog model
│   └── pricing.rs          -- Pricing model
```

### 10.2 Feature Flag

```toml
[features]
gateway = ["reqwest/stream"]
```

### 10.3 依赖

```toml
[dependencies]
reqwest = { version = "0.12", features = ["stream", "json"] }
async-stream = "0.3"
tokio-stream = "0.1"
```

### 10.4 性能目标（vs Go）

```
                    Go (one-api)      Rust (raisfast)
并发连接数          ~1,000            ~10,000+
内存占用            100-200MB         10-20MB
SSE 延迟抖动        GC 导致 ~50ms     <1ms
冷启动              ~1s               <50ms
二进制大小          ~50MB             ~10MB
单核 QPS            ~3,000            ~15,000+
```

### 10.5 与现有模块集成

```
gateway 模块复用:
  · users 表        → 用户管理
  · rbac            → 权限控制
  · audit_log       → 审计日志
  · api_tokens      → Key 管理（扩展 gateway_token）
  · plugin_system   → 格式转换可作为插件实现
  · worker/cron     → 渠道健康检查定时任务
  · webhook         → 用量告警
  · media           → 图片生成接口
  · tenant          → 多租户隔离
```

## 11. API 路由设计

### 11.1 中转接口（兼容 OpenAI）

```
# Chat
POST   /v1/chat/completions
POST   /v1/completions

# Embeddings
POST   /v1/embeddings

# Images
POST   /v1/images/generations
POST   /v1/images/edits
POST   /v1/images/variations

# Audio
POST   /v1/audio/transcriptions
POST   /v1/audio/translations
POST   /v1/audio/speech

# Models
GET    /v1/models

# Rerank (new-api)
POST   /v1/rerank
```

### 11.2 管理接口

```
# 渠道管理
GET    /api/v1/gateway/channels
POST   /api/v1/gateway/channels
PUT    /api/v1/gateway/channels/:id
DELETE /api/v1/gateway/channels/:id
POST   /api/v1/gateway/channels/:id/test

# Key 管理
GET    /api/v1/gateway/tokens
POST   /api/v1/gateway/tokens
PUT    /api/v1/gateway/tokens/:id
DELETE /api/v1/gateway/tokens/:id

# 定价管理
GET    /api/v1/gateway/pricing
POST   /api/v1/gateway/pricing
PUT    /api/v1/gateway/pricing/:id

# 统计
GET    /api/v1/gateway/logs
GET    /api/v1/gateway/stats/overview
GET    /api/v1/gateway/stats/models
GET    /api/v1/gateway/stats/channels

# 用户分组
GET    /api/v1/gateway/groups
PUT    /api/v1/gateway/groups/:id
```

## 12. 与 one-api/new-api 的差异化

```
one-api/new-api 的不足              raisfast 的改进
─────────────────────               ────────────────
Go GC 导致 SSE 抖动               无 GC，延迟稳定
单机架构，扩展靠 Redis             多租户原生支持
格式转换硬编码                     插件化 adaptor
定价硬编码或单表                   灵活表达式定价
无桌面端                           Tauri 桌面管理
无 Content Type                    可组合内容建模
无工作流                           工作流引擎
无搜索                             Tantivy 全文搜索
```
