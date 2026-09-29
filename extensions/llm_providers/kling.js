// Kling AI (可灵) provider 扩展 —— 自原生
// crates/core/src/llm/providers/kling.rs 迁移（逻辑逐字段对照原生实现与
// 其测试断言；JWT 签名消费 utils.jwtSignHS256，对照 new-api
// plugins/tasks/kling/plugin.js:105-106）。
//
// 双协议（2026-09，key 形态即协议选择器）：
// - `access_key:secret_key` 双段 → 旧版 API（api.klingai.com，per-request
//   JWT，/v1/videos/text2video，body.model_name）[照抄原生 kling.rs]；
// - 单段 API Key（klingai.com/dev 新版开发者平台，`api-key-kling-*`）→
//   新版 API（api-beijing.klingai.com，Bearer 透传，/text-to-video/{model}，
//   body.settings/options）[照抄 klingai.com/document-api llms.md]。
//   证据链：api-singapore 报 "api key not found"（按 key 查表）→ 新端点
//   api-beijing 鉴权通过（报模型下线而非鉴权失败）。
//
// 共同点：任务查询双路径（taskData 持久化提交端点 action §4.2/§4.3），
// 无 404 探测（新版统一 /tasks，无需 action）。

export const meta = {
  key: "kling", // llm_channels.provider 的匹配键
  name: "Kling AI (可灵)",
  version: "1.1.0",
  contract: 1,
  protocols: ["video"],
  models: ["kling-3.0", "kling-3.0-turbo", "kling-v1-6", "kling-v1-5", "kling-v1"],
  description:
    "可灵文生/图生视频。新版（dev 平台 API Key）：base_url=api-beijing.klingai.com，模型 kling-3.0[-turbo]，首尾帧；旧版（ak:sk）：base_url=api.klingai.com，JWT 鉴权",
  http: [
    "api-beijing.klingai.com/*",
    "api.klingai.com/*",
    "api-singapore.klingai.com/*",
  ],
  timeout_ms: 30000,
};

// key 形态即协议选择器：单段 = 新版 dev 平台 API Key。
function isNewApi(apiKey) {
  return !String(apiKey ?? "").includes(":");
}

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
// 单段 key 透传（[自造-兼容] 2026-09）：klingai.com/dev/api-key 新版开发
// 平台签发 `api-key-kling-*` 单段 API Key（Bearer 直传，无 ak/sk 对）——
// 上游 api-singapore 对该形态报 "api key not found"（按 key 查表），证明
// Bearer 透传是其认可的鉴权形态。ak:sk 双段仍走原 JWT 路径，行为不变。
function bearerToken(apiKey) {
  const raw = String(apiKey ?? "");
  if (!raw.includes(":")) {
    if (!raw.trim()) throw new Error("kling: key must be `access_key:secret_key`");
    return raw;
  }
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

// 提交：按 key 形态分派协议。无参考图 → 文生视频；有 → 图生视频
// （首帧 = 首个参考图）。渠道 paramOverride 浅合并进 body 顶层。
export function buildSubmitRequest(ctx) {
  if (!String(ctx.apiKey ?? "").trim()) {
    throw new Error("kling: key must be `access_key:secret_key`");
  }
  return isNewApi(ctx.apiKey) ? submitNew(ctx) : submitLegacy(ctx);
}

// ── 新版协议（klingai.com/dev API Key → api-beijing.klingai.com）──────
// [照抄 https://klingai.com/document-api/api/video/3-0-omni/text-to-video.md]
// 路径内嵌模型：/text-to-video/{model} | /image-to-video/{model}；
// body = {prompt, settings:{duration:int 3-15, aspect_ratio, resolution…},
//         options:{callback_url,…}}；i2v 用 contents[]{prompt,first_frame}。

function submitNew(ctx) {
  const refs = (ctx.request.inputReferences || []).map(toWireImage).filter((s) => s.length > 0);

  const settings = {};
  // seconds 是 wire 字符串直传（trait 约定）→ 新版要 int。
  const duration = Number(ctx.request.seconds);
  if (duration > 0) settings.duration = duration;
  const aspect = aspectRatioFromSize(ctx.request.size);
  if (aspect) settings.aspect_ratio = aspect;

  const isI2V = refs.length > 0;
  const body = isI2V
    ? {
        contents: [
          { type: "prompt", text: String(ctx.request.prompt ?? "") },
          ...refs.map((url) => ({ type: "first_frame", url })),
        ],
      }
    : { prompt: String(ctx.request.prompt ?? "") };
  if (Object.keys(settings).length > 0) body.settings = settings;
  if (ctx.request.callbackUrl) body.options = { callback_url: ctx.request.callbackUrl };

  if (ctx.paramOverride && typeof ctx.paramOverride === "object") {
    Object.assign(body, ctx.paramOverride);
  }

  const path = (isI2V ? "/image-to-video/" : "/text-to-video/") + ctx.model;
  return {
    url: ctx.baseUrl + path,
    method: "POST",
    headers: { Authorization: "Bearer " + String(ctx.apiKey) },
    body,
  };
}

// ── 旧版协议（ak:sk → api.klingai.com JWT）[照抄原生 kling.rs] ────────

function submitLegacy(ctx) {
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

// 提交响应 → 任务句柄 + taskData（旧版：提交端点 action 供查询选路；
// 新版统一 /tasks，无需 action，仅存协议标记）。
export function parseSubmitResponse(ctx, response) {
  const data = envelope(ctx, response);
  if (isNewApi(ctx.apiKey)) {
    // data: {id, status: submitted|processing|succeeded|failed}
    const taskId = String(data.id ?? "");
    if (!taskId) throw new Error("kling new-api: envelope without data.id");
    const state = String(data.status || "");
    const status =
      state === "succeeded" ? "completed" : state === "failed" ? "failed" : state === "submitted" ? "queued" : "in_progress";
    return { taskId, status, taskData: { api: "new" } };
  }
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

// 轮询：旧版 taskData.action 确定性选端点（无 404 探测）；新版统一
// GET /tasks?task_ids=（批量接口，单查即单元素）。
export function buildQueryRequest(ctx) {
  if (isNewApi(ctx.apiKey)) {
    return {
      url: ctx.baseUrl + "/tasks?task_ids=" + encodeURIComponent(String(ctx.taskId ?? "")),
      method: "GET",
      headers: {
        Authorization: "Bearer " + String(ctx.apiKey),
        "Content-Type": "application/json",
      },
    };
  }
  const action = (ctx.taskData && ctx.taskData.action) || "text2video";
  return {
    url: ctx.baseUrl + "/v1/videos/" + action + "/" + ctx.taskId,
    method: "GET",
    headers: { Authorization: "Bearer " + bearerToken(ctx.apiKey) },
  };
}

// 轮询响应 → 任务快照。旧版 [照抄原生 status_from_wire + task_status_msg]；
// 新版 data 为任务数组（取本任务），succeeded 时从 outputs[] 取视频 url，
// failed 的原因在 message 字段；unknown → in_progress（sweep 容忍）。
export function parseTaskResult(ctx, response) {
  const data = envelope(ctx, response);
  if (isNewApi(ctx.apiKey)) {
    const tasks = Array.isArray(data) ? data : [data];
    const task =
      tasks.find((t) => String((t && t.id) ?? "") === String(ctx.taskId ?? "")) || tasks[0] || {};
    const state = String(task.status || "");
    if (state === "succeeded") {
      const video = (task.outputs || []).find((o) => o && o.type === "video") || {};
      return video.url ? { status: "completed", url: String(video.url) } : { status: "completed" };
    }
    if (state === "failed") {
      return { status: "failed", error: String(task.message || "task failed") };
    }
    return { status: "in_progress" };
  }
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
