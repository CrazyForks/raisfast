// CogVideo (智谱) provider 扩展 —— provider-plugins.md §4.6 试点。
//
// 参考来源：智谱开放平台公开文档（open.bigmodel.cn，videos/generations
// 异步任务接口）；third/ 无源码参考（设计 §4.6 已标注）。
//
// 契约（§4.2）：meta + 四个纯函数；build (ctx)、parse (ctx, payload)；
// 参数为真实 JS 对象，无 I/O —— HTTP 由宿主代发（§5.3）。

export const meta = {
  key: "cogvideo", // llm_channels.provider 的匹配键
  name: "CogVideo (智谱)",
  version: "0.1.0",
  contract: 1,
  protocols: ["video"],
  // 建议模型清单（UX 预填；权威源 = llm_models 目录，非定价依据）。
  models: ["cogvideox-2", "cogvideox", "cogvideox-flash"],
  description: "智谱 CogVideoX 文生视频/图生视频（异步任务）",
  // host 代发前逐请求校验；成片下载在宿主侧、不走本清单（§5.3）。
  http: ["open.bigmodel.cn/*"],
  timeout_ms: 30000,
};

// 提交：POST /videos/generations，Bearer 鉴权。
// ctx.request.{prompt, seconds, size, inputReferences}，ctx.paramOverride 透传。
export function buildSubmitRequest(ctx) {
  const duration = Number.parseInt(ctx.request.seconds ?? "", 10);
  const body = {
    model: ctx.model,
    prompt: ctx.request.prompt,
    ...(Number.isFinite(duration) && duration > 0 ? { duration } : {}),
    ...(ctx.paramOverride && typeof ctx.paramOverride === "object" ? ctx.paramOverride : {}),
  };
  return {
    url: ctx.baseUrl + "/api/paas/v4/videos/generations",
    method: "POST",
    headers: { Authorization: "Bearer " + ctx.apiKey },
    body,
  };
}

// 提交响应 → 任务句柄。Zhipu：200 + { id | task_id, task_status }。
export function parseSubmitResponse(ctx, response) {
  const body = response.body || {};
  if (response.status >= 400) {
    throw new Error("cogvideo submit failed: " + JSON.stringify(body).slice(0, 200));
  }
  const taskId = String(body.id ?? body.task_id ?? "");
  if (!taskId) {
    throw new Error("cogvideo submit response has no task id: " + JSON.stringify(body).slice(0, 200));
  }
  return { taskId };
}

// 轮询：GET /videos/generations/{taskId}，Bearer 鉴权。
export function buildQueryRequest(ctx) {
  return {
    url: ctx.baseUrl + "/api/paas/v4/videos/generations/" + ctx.taskId,
    method: "GET",
    headers: { Authorization: "Bearer " + ctx.apiKey },
  };
}

// 轮询响应 → 任务快照。
// Zhipu task_status：PROCESSING / SUCCESS / FAIL；成片在 video_result[0].url。
export function parseTaskResult(ctx, response) {
  const body = response.body || {};
  const state = String(body.task_status || "").toUpperCase();
  if (state === "SUCCESS") {
    const url = body.video_result?.[0]?.url || "";
    return { status: "completed", url };
  }
  if (state === "FAIL") {
    return { status: "failed", error: "cogvideo task failed" };
  }
  // PROCESSING 及未知状态一律 in-progress（sweep 容忍语义，§5.2）。
  return { status: "in_progress" };
}
