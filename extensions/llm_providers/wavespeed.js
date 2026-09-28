// WaveSpeed provider 扩展 —— 自原生 third/MoneyPrinterTurbo
// app/services/material.py `generate_videos_wavespeed` /
// `_wait_for_wavespeed_prediction` 迁移（协议逐字段对照）。
//
// WaveSpeed 异步文生视频：模型路由在 URL 路径上（POST /{model_id}），
// 信封 {code:200, data:{id,status,outputs}}；轮询 GET /predictions/{id}/result。
//
// 付费任务纪律 [照抄原生]：提交 POST 绝不自动重试（可能已创建付费任务）；
// data.status ∈ {failed, cancelled, timeout} → 该关键词无产物（跳过继续）。

export const meta = {
  key: "wavespeed", // llm_channels.provider 的匹配键
  name: "WaveSpeed",
  version: "1.0.0",
  contract: 1,
  protocols: ["video"],
  models: ["bytedance/seedance-2.0-fast/text-to-video"],
  description: "WaveSpeed 聚合网关文生视频（模型路由在 URL 路径；4-15s）",
  http: ["api.wavespeed.ai/*"],
  timeout_ms: 30000,
};

// [照抄原生 WAVESPEED_MIN/MAX_DURATION_SECONDS = 4..15] —— 超出会被 API
// 直接拒绝；可经 paramOverride.minDuration/maxDuration 校准（不同模型区间
// 不同，如部分为 2..15）。
const MIN_DURATION = 4;
const MAX_DURATION = 15;

// 宽高比：显式 paramOverride.aspect_ratio 优先；否则按 size 宽高比就近
// 两档（16:9 / 9:16）；缺省 9:16（MPT 竖屏缺省）。
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

// 提交：POST {base}/{model_id}，Bearer 鉴权，body {prompt, aspect_ratio,
// duration}。model_id 即 URL 路由段（ctx.model，含 `/` 分隔的模型路径）。
export function buildSubmitRequest(ctx) {
  const prompt = String(ctx.request.prompt ?? "").trim();
  if (!prompt) throw new Error("wavespeed: prompt must not be empty");

  const duration = Number(String(ctx.request.seconds ?? "").trim());
  const clamped = Number.isInteger(duration)
    ? Math.min(Math.max(duration, MIN_DURATION), MAX_DURATION)
    : MIN_DURATION;

  return {
    url: ctx.baseUrl + "/" + ctx.model,
    method: "POST",
    headers: { Authorization: "Bearer " + ctx.apiKey },
    body: {
      prompt,
      aspect_ratio: aspectRatio(ctx),
      duration: clamped,
    },
  };
}

// 提交响应 → 任务句柄。
// [照抄原生：envelope code !== 200 → 明确拒绝（4xx/业务码，无重复计费
// 风险，返回空结果继续）；5xx → unconfirmed（可能已计费，不盲重试）]。
export function parseSubmitResponse(ctx, response) {
  const body = response.body || {};
  if (body.code !== 200) {
    throw new Error(
      `wavespeed submit rejected: http_status=${response.status}, code=${body.code ?? "null"}, detail=${body.message ?? ""}`,
    );
  }
  const data = body.data || {};
  const taskId = String(data.id ?? "").trim();
  if (!taskId) throw new Error("wavespeed submit response has no prediction id");
  return { taskId };
}

// 轮询：GET /predictions/{prediction_id}/result。
export function buildQueryRequest(ctx) {
  return {
    url: ctx.baseUrl + "/predictions/" + ctx.taskId + "/result",
    method: "GET",
    headers: { Authorization: "Bearer " + ctx.apiKey },
  };
}

// 轮询响应 → 任务快照。
// [照抄原生 _wait_for_wavespeed_prediction：envelope code !== 200 → 状态
// 未知；data.status completed → 成功（outputs 为 URL 数组）；failed/
// cancelled/timeout → 失败；其余 → in_progress]。
export function parseTaskResult(ctx, response) {
  const body = response.body || {};
  if (body.code !== 200) {
    throw new Error(
      "wavespeed prediction status unknown: http_status=" +
        (response.status ?? "?") +
        ", code=" +
        (body.code ?? "null") +
        ", detail=" +
        (body.message ?? ""),
    );
  }
  const data = body.data || {};
  const state = String(data.status || "");
  if (state === "completed") {
    const outputs = Array.isArray(data.outputs) ? data.outputs : [];
    return outputs.length > 0
      ? { status: "completed", url: outputs[0] }
      : { status: "completed" };
  }
  if (state === "failed" || state === "cancelled" || state === "timeout") {
    return { status: "failed", error: "wavespeed task " + state };
  }
  return { status: "in_progress" };
}
