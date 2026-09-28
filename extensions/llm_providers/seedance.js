// Seedance (火山方舟/即梦) provider 扩展 —— 自原生
// crates/core/src/llm/providers/seedance.rs 迁移（逻辑逐字段对照原生实现
// 与其测试断言；协议参照 MPT volcengine_seedance + Ark 公开文档）。
//
// 火山方舟 Ark 协议：POST /contents/generations/tasks 提交，同路径 GET
// 轮询；content 数组（text + 可选 image_url 首帧锚定）；resolution 从渠道
// param_override 读取并硬校验（无效报错不静默回退，防超预期费用）。

export const meta = {
  key: "seedance", // llm_channels.provider 的匹配键
  name: "Seedance (火山方舟/即梦)",
  version: "1.0.0",
  contract: 1,
  protocols: ["video"],
  // 建议模型清单（UX 预填；权威源 = llm_models 目录）。
  models: ["doubao-seedance-1-0-pro", "doubao-seedance-1-0-lite-t2v", "doubao-seedance-1-0-lite-i2v"],
  description: "火山方舟 Seedance 文生视频/图生视频（首帧锚定；华北 region）",
  // 其他 Ark region 的部署：在此追加 host pattern 后 reload。
  http: ["ark.cn-beijing.volces.com/*"],
  timeout_ms: 30000,
};

const RESOLUTIONS = ["480p", "720p", "1080p"];
// [照抄原生 MIN/MAX_DURATION_SECS = 2..12]
const MIN_DURATION = 2;
const MAX_DURATION = 12;

// [照抄原生 providers::aspect_ratio_from_size 共享版，三档最近邻] ——
// size 缺省 = "9:16"（MPT 默认竖屏）。
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
  if (!ref) return undefined;
  if (ref.url) return ref.url;
  if (ref.b64Json) return `data:${ref.mime || "image/png"};base64,${ref.b64Json}`;
  return undefined;
}

// [照抄原生 resolution()] —— 渠道 param_override.resolution 硬校验：
// 无效值报错而非静默回退，防超预期费用；缺省 1080p。
function resolution(paramOverride) {
  const raw = String(
    (paramOverride && typeof paramOverride === "object" ? paramOverride.resolution : "") ?? "1080p",
  )
    .trim()
    .toLowerCase();
  if (RESOLUTIONS.includes(raw)) return raw;
  throw new Error(
    `unsupported seedance resolution "${raw}"; expected one of: ${RESOLUTIONS.join(", ")}`,
  );
}

// [照抄原生 duration_seconds] —— INT clamp 2..12；非法/缺失回落 2。
function durationSeconds(seconds) {
  const raw = String(seconds ?? "").trim();
  const n = /^[0-9]+$/.test(raw) ? Number(raw) : MIN_DURATION;
  return Math.min(Math.max(n, MIN_DURATION), MAX_DURATION);
}

// 提交：content 数组（text + 可选 image_url 首帧）。
export function buildSubmitRequest(ctx) {
  const prompt = String(ctx.request.prompt ?? "");
  // 空提示词守卫 [照抄 MPT generate_videos 空词守卫 —— 付费源不提交空任务]。
  if (!prompt.trim()) throw new Error("seedance: prompt must not be empty");

  const content = [{ type: "text", text: prompt }];
  const wire = toWireImage((ctx.request.inputReferences || [])[0]);
  if (wire) {
    // 图生视频：首个参考图锚定首帧 [参考 Ark 公开文档 —— MPT 仅覆盖文生视频]。
    content.push({ type: "image_url", image_url: { url: wire }, role: "first_frame" });
  }

  const paramOverride = ctx.paramOverride && typeof ctx.paramOverride === "object" ? ctx.paramOverride : {};
  const body = {
    model: ctx.model,
    content,
    // ratio 取参考的画布枚举；size 缺省 = 9:16（MPT 默认竖屏）。
    ratio: aspectRatioFromSize(ctx.request.size) ?? "9:16",
    duration: durationSeconds(ctx.request.seconds),
    resolution: resolution(paramOverride),
    watermark: false,
  };
  // 其余渠道覆盖（cfg_scale/camera_fixed/callback_url…）浅合并；
  // resolution 已校验应用，跳过 [照抄原生 skip-resolution 分支]。
  for (const [k, v] of Object.entries(paramOverride)) {
    if (k === "resolution") continue;
    body[k] = v;
  }

  return {
    url: ctx.baseUrl + "/contents/generations/tasks",
    method: "POST",
    headers: { Authorization: "Bearer " + ctx.apiKey },
    body,
  };
}

// 提交响应 → 任务句柄（body.id 必需；status 按任务态词汇映射）。
export function parseSubmitResponse(ctx, response) {
  const body = response.body || {};
  const taskId = String(body.id ?? "");
  if (!taskId) throw new Error("seedance: task response without id");
  const state = String(body.status || "");
  const status =
    state === "queued"
      ? "queued"
      : state === "succeeded"
        ? "completed"
        : state === "failed" || state === "cancelled" || state === "canceled" || state === "expired"
          ? "failed"
          : "in_progress";
  return { taskId, status };
}

// 轮询：GET /contents/generations/tasks/{taskId}。
export function buildQueryRequest(ctx) {
  return {
    url: ctx.baseUrl + "/contents/generations/tasks/" + ctx.taskId,
    method: "GET",
    headers: { Authorization: "Bearer " + ctx.apiKey },
  };
}

// 轮询响应 → 任务快照。
// [照抄原生 status_from_wire：unknown → in_progress（sweep 容忍）；
//  成片 content.video_url；error 字段为上游错误文本（宿主已 redact）]。
export function parseTaskResult(ctx, response) {
  const body = response.body || {};
  const state = String(body.status || "");
  if (state === "succeeded") {
    const url = String((body.content || {}).video_url || "");
    return url ? { status: "completed", url } : { status: "completed" };
  }
  if (state === "queued") return { status: "queued" };
  if (state === "failed" || state === "cancelled" || state === "canceled" || state === "expired") {
    return { status: "failed", error: String(body.error || state) };
  }
  return { status: "in_progress" };
}
