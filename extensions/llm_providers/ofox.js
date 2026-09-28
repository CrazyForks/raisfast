// OFox 视频聚合网关 provider 扩展 —— 自原生
// third/MoneyPrinterTurbo/app/services/ofox.py 迁移（协议逐字段对照）。
//
// 聚合网关：多厂商通道（bytedance/seedance-2.0-fast、alibaba/wan-2.7…），
// provider.type 显式钉定通道 [照抄原生 DEFAULT_PROVIDER_TYPE = "byteplus"]
// —— 面向全球受众时内容政策更一致；可经渠道 paramOverride.providerType
// 切换。
//
// 付费任务纪律 [照抄原生]：提交超时/5xx 可能发生在付费任务已创建之后，
// 盲目重试会重复扣费——4xx 才是明确拒绝。成片 URL 优先 mirror_urls
// （OFox CDN 持久签名），回退 unsigned_urls（上游临时直链，~24h 过期）。

export const meta = {
  key: "ofox", // llm_channels.provider 的匹配键
  name: "OFox (视频聚合网关)",
  version: "1.0.0",
  contract: 1,
  protocols: ["video"],
  models: ["bytedance/seedance-2.0-fast", "alibaba/wan-2.7"],
  description: "OFox 聚合网关文生视频（Bearer 鉴权；provider.type 钉定厂商通道）",
  http: ["ofox.ai/*"],
  timeout_ms: 30000,
};

// seedance-2.0-fast 服务端实测校验 4-15s；其他模型区间不同，切换模型时
// 经 paramOverride 校准 [照抄原生 DEFAULT_MIN/MAX_DURATION_SECONDS]。
const MIN_DURATION = 4;
const MAX_DURATION = 15;
const FAILURE_STATES = ["failed", "error", "cancelled", "canceled", "expired"];

// 宽高比：显式 paramOverride.aspect_ratio 优先；否则按 size 就近三档；
// 缺省 9:16（MPT 竖屏缺省）。
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

// 提交：POST {base}/videos，Bearer 鉴权。
// body: {model, prompt, duration(clamp 4..15), resolution, aspect_ratio,
//        provider: {type}}。
export function buildSubmitRequest(ctx) {
  const prompt = String(ctx.request.prompt ?? "").trim();
  if (!prompt) throw new Error("ofox: prompt must not be empty");

  const overrides = ctx.paramOverride && typeof ctx.paramOverride === "object" ? ctx.paramOverride : {};
  const model = String(overrides.model ?? "bytedance/seedance-2.0-fast").trim();
  const resolution = String(overrides.resolution ?? "720p").trim();
  const seconds = Number(String(ctx.request.seconds ?? "").trim());
  const duration = Number.isInteger(seconds)
    ? Math.min(Math.max(seconds, MIN_DURATION), MAX_DURATION)
    : MIN_DURATION;

  const body = {
    model,
    prompt,
    duration,
    resolution,
    aspect_ratio: aspectRatio(ctx),
    provider: { type: String(overrides.providerType ?? "byteplus") },
  };

  return {
    url: ctx.baseUrl + "/videos",
    method: "POST",
    headers: { Authorization: "Bearer " + ctx.apiKey },
    body,
  };
}

// 提交响应 → 任务句柄（body.id 必需——OFox 接受提交后必回任务 id）。
export function parseSubmitResponse(ctx, response) {
  const body = response.body || {};
  const taskId = String(body.id ?? "").trim();
  if (!taskId) throw new Error("ofox: accepted the submission without returning a task id");
  return { taskId };
}

// 轮询：GET {base}/videos/{taskId}。
export function buildQueryRequest(ctx) {
  return {
    url: ctx.baseUrl + "/videos/" + ctx.taskId,
    method: "GET",
    headers: { Authorization: "Bearer " + ctx.apiKey },
  };
}

// 轮询响应 → 任务快照。
// [照抄原生状态机：pending(收单待上游提交) → queued → in_progress →
//  completed；failed/error/cancelled/canceled/expired → failed。
//  成片 URL 优先 mirror_urls（CDN 持久签名）回退 unsigned_urls（临时直链）]。
export function parseTaskResult(ctx, response) {
  const body = response.body || {};
  const state = String(body.status ?? "").trim().toLowerCase();

  if (state === "completed" || state === "succeeded") {
    let url = "";
    for (const field of ["mirror_urls", "unsigned_urls"]) {
      const urls = body[field];
      for (const candidate of Array.isArray(urls) ? urls : []) {
        if (typeof candidate === "string" && candidate.startsWith("http")) {
          url = candidate;
          break;
        }
      }
      if (url) break;
    }
    return url
      ? { status: "completed", url }
      : { status: "failed", error: `ofox: task completed without a downloadable video` };
  }
  if (FAILURE_STATES.includes(state)) {
    const err = body.error;
    const detail = typeof err === "object" && err !== null
      ? String(err.message || err.detail || "")
      : String(body.message || "");
    return { status: "failed", error: detail || state };
  }
  return { status: "in_progress" };
}
