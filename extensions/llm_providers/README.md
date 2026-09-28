# LLM Provider 扩展开发指南

一个 `.js` 文件 = 一个视频生成厂商扩展。改文件 → 跑测试 → reload，零编译零发版。
完整设计：`dev-docs/llm/provider-plugins.md`。

## 文件约定

```
extensions/llm_providers/
├── package.json        # {"type":"module"}（共享，勿删）
├── <vendor>.js         # 一个文件一个扩展：meta + 四个契约函数
└── test/
    └── <vendor>.test.mjs
```

## 扩展骨架

```js
export const meta = {
  key: "myvendor",              // llm_channels.provider 匹配键（全局唯一）
  name: "MyVendor",
  version: "1.0.0",
  contract: 1,                  // 内核契约版本
  protocols: ["video"],
  models: ["model-a", "model-b"], // 建议清单（UX 预填，权威源 = llm_models 目录）
  description: "一句话描述",
  http: ["api.myvendor.com/*"], // 出网白名单（host 代发前逐请求校验）
  timeout_ms: 30000,
};

// build 类单参 (ctx)，parse 类 (ctx, payload)。参数是真实 JS 对象。
export function buildSubmitRequest(ctx) {
  // ctx: {provider, baseUrl, apiKey, model, paramOverride,
  //       request: {prompt, seconds, size, inputReferences, callbackUrl}}
  // 返回 {url, method?, headers?, body?}
}

export function parseSubmitResponse(ctx, response) {
  // response: {status, body} —— 只会收到 2xx（非 2xx 宿主已报 Http 错误）
  // 返回 {taskId, status?, error?}
}

export function buildQueryRequest(ctx) { … }     // ctx.taskId 可用
export function parseTaskResult(ctx, response) {
  // 返回 {status: queued|in_progress|completed|failed, progress?, url?, error?}
  // url = 成片地址（宿主直接下载）；未知状态 → in_progress
}
```

## 可用全局：`utils`（纯计算，零 I/O）

| 函数 | 用途 |
|---|---|
| `utils.unixNow()` | 秒级时间戳（测试可注入固定值） |
| `utils.jwtSignHS256(claims, secret)` | per-request JWT（可灵类） |
| `utils.hmacSHA256(message, secret)` | hex 签名 |
| `utils.base64 / base64URL / base64URLDecode` | 编码 |
| `utils.uuid()` | nonce |

参考实现：`cogvideo.js`（最简）、`wan.js`（模型矩阵 + kind 路由）、`kling.js`（JWT + taskData 双路径）。

## 开发循环

```bash
$EDITOR extensions/llm_providers/myvendor.js
just test-llm-providers     # node --test，秒级
# 管理台 POST /admin/llm/providers/reload（或重启）
```

## 上线检查清单

- [ ] meta 完整（key 唯一、contract=1、http 非空、timeout ≥30000）
- [ ] 四个契约导出齐全（reload 时 fail-fast 校验）
- [ ] fixture 测试覆盖：正常提交/轮询 + 每种终态 + 信封错误 + 边界（空 prompt、越界时长）
- [ ] `parseTaskResult` 未知状态返回 `in_progress`（勿返回 failed）
- [ ] 付费档位硬校验写在扩展里（时长/分辨率组合）；host 侧另有 seconds 上限兜底
- [ ] key 格式约定写进 description（如可灵的 `access_key:secret_key`）

## 限制（契约 v1）

- 仅 GET/POST + JSON body（multipart 上传类厂商暂不支持）
- 仅非流式；无 crypto 之外的宿主计算原语（非对称签名厂商走原生）
- 成片下载由宿主直连（不走本清单、不带扩展 headers）
