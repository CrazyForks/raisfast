// Kling AI (可灵) provider 扩展 —— 自原生
// crates/core/src/llm/providers/kling.rs 迁移（逻辑逐字段对照原生实现与
// 其测试断言；JWT 签名消费 utils.jwtSignHS256，对照 new-api
// plugins/tasks/kling/plugin.js:105-106）。
//
// 关键差异（相对其他扩展）：
// - key 格式 `access_key:secret_key`（单字段双段，宿主注入 ctx.apiKey）；
// - per-request JWT（HS256，exp now+1800 / nbf now-5）；
// - 任务查询双路径：taskData 持久化提交端点 action（§4.2/§4.3）——查询
//   确定性选端点，无 404 探测（对照 new-api ctx.action 同款）。

export const meta = {
  key: "kling", // llm_channels.provider 的匹配键
  name: "Kling AI (可灵)",
  version: "1.0.0",
  contract: 1,
  protocols: ["video"],
  models: ["kling-v1-6", "kling-v1-5", "kling-v1"],
  description: "可灵文生视频/图生视频（首帧锚定；per-request JWT 鉴权）",
  http: ["api.klingai.com/*", "api-singapore.klingai.com/*"],
  timeout_ms: 30000,
};

// [照抄原生 kling.rs bearer：key = `access_key:secret_key`，首个 `:` 分割]
function splitKey(apiKey) {
  const raw = String(apiKey ?? "");
  const i = raw.indexOf(":");
  if (i <= 0 || i === raw.length - 1) {
    throw new Error("kling: key must be `access_key:secret_key`");
  }
  return [raw.slice(0, i).trim(), raw.slice(i + 1).trim()];
}

// [照抄原生 bearer JWT claims：iss=ak、exp=now+1800、nbf=now-5]
function bearerToken(apiKey) {
  const [ak, sk] = splitKey(apiKey);
  const now = utils.unixNow();
  return utils.jwtSignHS256({ iss: ak, exp: now + 1800, nbf: now - 5 }, sk);
}

// [照抄原生 aspect_ratio_from_size（共享版，三档最近邻）] —— 无法解析则
// 缺省（上游默认生效）。
function aspectRatioFromSize(size) {
  const parts = String(size ?? "").trim().toLowerCase().split("x");
  if (parts.length !== 2) return undefined;
  const w = Number(parts[0].trim());
  const h = Number(parts[1].trim());
  if (!(w > 0) || !(h > 0)) return undefined;
  const ratio = w / h;
  const candidates = [
    [16 / 9, "16:9"],
    [1, "1:1"],
    [9 / 16, "9:16"],
  ];
  let best = candidates[0];
  for (const c of candidates) {
    if (Math.abs(c[0] - ratio) < Math.abs(best[0] - ratio)) best = c;
  }
  return best[1];
}

// 参考图 → data-URL 或原 URL（[照抄本仓 VideoInputRef::to_wire_string]）。
function toWireImage(ref) {
  if (!ref) return "";
  if (ref.url) return ref.url;
  if (ref.b64Json) return `data:${ref.mime || "image/png"};base64,${ref.b64Json}`;
  return "";
}

// 提交：无参考图 → text2video；有 → image2video（首帧 = 首个参考图）。
// 渠道 paramOverride 浅合并进 body 顶层（mode/cfg_scale/…）。
export function buildSubmitRequest(ctx) {
  const refs = (ctx.request.inputReferences || []).map(toWireImage).filter((s) => s.length > 0);

  const body = {
    model_name: ctx.model,
    prompt: String(ctx.request.prompt ?? ""),
  };
  // duration 为 STRING [照抄 trait 注释：seconds 是 wire 字符串直传]
  if (ctx.request.seconds != null) body.duration = String(ctx.request.seconds);
  if (ctx.request.callbackUrl) body.callback_url = ctx.request.callbackUrl;
  const aspect = aspectRatioFromSize(ctx.request.size);
  if (aspect) body.aspect_ratio = aspect;
  if (refs.length > 0) body.image = refs[0];

  // 渠道 paramOverride 最后合并（优先级最高，同原生 merge_overrides）。
  if (ctx.paramOverride && typeof ctx.paramOverride === "object") {
    Object.assign(body, ctx.paramOverride);
  }

  const path = refs.length > 0 ? "/v1/videos/image2video" : "/v1/videos/text2video";
  return {
    url: ctx.baseUrl + path,
    method: "POST",
    headers: { Authorization: "Bearer " + bearerToken(ctx.apiKey) },
    body,
  };
}

// 信封 [照抄原生 unwrap_envelope]：{code, message, data}，code != 0 →
// 协议层错误（HTTP 层非 2xx 由宿主先行处理，parse 只见 2xx）。
function envelope(ctx, response) {
  const body = response.body || {};
  const code = Number(body.code ?? 0);
  if (code !== 0) {
    throw new Error(`kling code ${code}: ${body.message || ""}`);
  }
  return body.data || {};
}

// 提交响应 → 任务句柄 + taskData（提交端点 action，查询时确定性选端点）。
export function parseSubmitResponse(ctx, response) {
  const data = envelope(ctx, response);
  const taskId = String(data.task_id ?? "");
  if (!taskId) throw new Error("kling: envelope without data.task_id");
  const state = String(data.task_status || "");
  const status =
    state === "submitted" ? "queued" : state === "succeed" ? "completed" : state === "failed" ? "failed" : "in_progress";
  const action = (ctx.request.inputReferences || []).filter((r) => r && (r.url || r.b64Json)).length > 0
    ? "image2video"
    : "text2video";
  return { taskId, status, taskData: { action } };
}

// 轮询：taskData.action 确定性选端点（提交期回传，无 404 探测）。
export function buildQueryRequest(ctx) {
  const action = (ctx.taskData && ctx.taskData.action) || "text2video";
  return {
    url: ctx.baseUrl + "/v1/videos/" + action + "/" + ctx.taskId,
    method: "GET",
    headers: { Authorization: "Bearer " + bearerToken(ctx.apiKey) },
  };
}

// 轮询响应 → 任务快照 [照抄原生 status_from_wire + task_status_msg；
// unknown → in_progress（sweep 容忍）]。
export function parseTaskResult(ctx, response) {
  const data = envelope(ctx, response);
  const state = String(data.task_status || "");
  if (state === "succeed") {
    const url = ((((data.task_result || {}).videos) || [])[0] || {}).url || "";
    return url ? { status: "completed", url } : { status: "completed" };
  }
  if (state === "failed") {
    return { status: "failed", error: String(data.task_status_msg || "task failed") };
  }
  return { status: "in_progress" };
}
