// MuAPI 视频聚合网关 provider 扩展 —— 自原生
// third/MoneyPrinterTurbo/app/services/muapi.py 迁移（协议逐字段对照）。
//
// 聚合网关：一个 key 覆盖多家模型（endpoint 路由，默认 seedance-lite-t2v，
// 可经渠道 paramOverride.endpoint 切换）。付费任务纪律 [照抄原生]：
// 提交超时/5xx 可能意味着任务已被计费创建——桥接层 Transport/Http 错误
// 不应盲目重试同渠道（内核 failover 到其他渠道安全）。

export const meta = {
  key: "muapi", // llm_channels.provider 的匹配键
  name: "MuAPI (视频聚合网关)",
  version: "1.0.0",
  contract: 1,
  protocols: ["video"],
  // 建议模型清单（实为 endpoint 路由；UX 预填）。
  models: ["seedance-lite-t2v"],
  description: "MuAPI 聚合网关文生视频（x-api-key 鉴权；endpoint 可经 paramOverride 切换）",
  http: ["api.muapi.ai/*"],
  timeout_ms: 30000,
};

const MIN_DURATION = 3;
const MAX_DURATION = 12;
const DEFAULT_ENDPOINT = "seedance-lite-t2v";
const DEFAULT_RESOLUTION = "480p";
const FAILURE_STATES = ["failed", "cancelled", "canceled", "expired"];

// 宽高比：显式 paramOverride.aspect_ratio 优先；否则按 size 宽高比就近
// 三档（16:9/9:16/1:1）；缺省 9:16（MPT 竖屏缺省）。
function aspectRatio(ctx) {
  const explicit = String((ctx.paramOverride || {}).aspect_ratio ?? "").trim();
  if (explicit) return explicit;
  const parts = String(ctx.request.size ?? "").trim().toLowerCase().split("x");
  if (parts.length === 2) {
    const w = Number(parts[0]);
    const h = Number(parts[1]);
    if (w > 0 && h > 0) {
      const r = w / h;
      const candidates = [
        [16 / 9, "16:9"],
        [9 / 16, "9:16"],
        [1, "1:1"],
      ];
      let best = candidates[0];
      for (const c of candidates) {
        if (Math.abs(c[0] - r) < Math.abs(best[0] - r)) best = c;
      }
      return best[1];
    }
  }
  return "9:16";
}

// 提交：POST {base}/{endpoint}，x-api-key 鉴权。
// body: {prompt, aspect_ratio, resolution, duration}（clamp 3..12）。
export function buildSubmitRequest(ctx) {
  const prompt = String(ctx.request.prompt ?? "").trim();
  if (!prompt) throw new Error("muapi: prompt must not be empty");

  const endpoint = String((ctx.paramOverride || {}).endpoint ?? DEFAULT_ENDPOINT).trim();
  const resolution = String((ctx.paramOverride || {}).resolution ?? DEFAULT_RESOLUTION).trim();
  const seconds = Number(String(ctx.request.seconds ?? "").trim());
  const duration = Number.isInteger(seconds)
    ? Math.min(Math.max(seconds, MIN_DURATION), MAX_DURATION)
    : MIN_DURATION;

  return {
    url: ctx.baseUrl + "/" + endpoint,
    method: "POST",
    headers: { "x-api-key": ctx.apiKey },
    body: {
      prompt,
      aspect_ratio: aspectRatio(ctx),
      resolution,
      duration,
    },
  };
}

// 提交响应 → 任务句柄 [照抄原生：request_id 优先、id 兜底，缺失报错——
// MuAPI 接受提交后必回请求 id，缺失意味着付费任务状态未知]。
export function parseSubmitResponse(ctx, response) {
  const body = response.body || {};
  const taskId = String(body.request_id ?? body.id ?? "").trim();
  if (!taskId) {
    throw new Error("muapi: accepted the submission without returning a request id");
  }
  return { taskId };
}

// 轮询：GET {base}/predictions/{taskId}/result。
export function buildQueryRequest(ctx) {
  return {
    url: ctx.baseUrl + "/predictions/" + ctx.taskId + "/result",
    method: "GET",
    headers: { "x-api-key": ctx.apiKey },
  };
}

// 轮询响应 → 任务快照 [照抄原生状态词汇：queued/pending/processing 活跃、
// completed 成功、failed/cancelled/canceled/expired 失败；未知 → in_progress]。
export function parseTaskResult(ctx, response) {
  const body = response.body || {};
  const state = String(body.status ?? "").trim().toLowerCase();

  if (state === "completed") {
    // outputs 可为单个 URL 字符串或 URL 数组（取第一个）。
    const outputs = body.outputs;
    const url = typeof outputs === "string"
      ? outputs
      : Array.isArray(outputs)
        ? String(outputs[0] ?? "")
        : "";
    return url ? { status: "completed", url } : { status: "completed" };
  }
  if (FAILURE_STATES.includes(state)) {
    const err = body.error;
    const detail = typeof err === "object" && err !== null
      ? err.message || err.detail || ""
      : String(err ?? "");
    return { status: "failed", error: detail || state };
  }
  return { status: "in_progress" };
}
