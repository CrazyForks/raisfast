// Vidu (生数) provider 扩展 —— 自原生 crates/core/src/llm/providers/vidu.rs
// 迁移（首试点）。执行模型照抄 inline-script runner；逻辑逐字段对照原生
// Rust 实现与其测试断言；协议参照 new-api plugins/tasks/vidu/plugin.js。
//
// 迁移记录（provider-plugins.md）：首个原生→JS 迁移厂商。

export const meta = {
  key: "vidu", // llm_channels.provider 的匹配键
  name: "Vidu (生数)",
  version: "1.0.0",
  contract: 1,
  protocols: ["video"],
  // 建议模型清单（UX 预填；权威源 = llm_models 目录，非定价依据）。
  models: ["viduq2", "viduq1", "vidu2.0", "vidu1.5"],
  description: "生数 Vidu 文生视频/图生视频/首尾帧（异步任务）",
  // 自定义网关/代理的部署：在此追加 host pattern 后 reload。
  http: ["api.vidu.cn/*", "api.vidu.com/*"],
  timeout_ms: 30000,
};

const RESOLUTIONS = ["360p", "540p", "720p", "1080p"];

// [照抄 vidu/plugin.js isQ2Model] —— viduq2* 前缀
function isQ2(model) {
  return String(model).indexOf("viduq2") === 0;
}

// [照抄 new-api defaultDuration/defaultResolution]
function defaultDuration(model) {
  return model === "vidu2.0" ? 4 : 5;
}
function defaultResolution(model) {
  return isQ2(model) ? "720p" : model === "vidu2.0" ? "360p" : "1080p";
}

// [照抄 new-api normalizeResolution] —— viduq1 锁 1080p；直接档位；
// WxH/W*H 按最长边归档（1920→1080p、1280→720p、960→540p、其余 360p）。
function normalizeResolution(value, model) {
  if (model === "viduq1") return "1080p";
  const raw = String(value ?? "").trim().toLowerCase();
  if (RESOLUTIONS.includes(raw)) return raw;
  const parts = raw.replace(/\*/g, "x").split("x");
  if (parts.length === 2) {
    const w = Number(parts[0].trim());
    const h = Number(parts[1].trim());
    if (w > 0 && h > 0) {
      const max = Math.max(w, h);
      return max >= 1920 ? "1080p" : max >= 1280 ? "720p" : max >= 960 ? "540p" : "360p";
    }
  }
  return defaultResolution(model);
}

// [照抄 new-api validateViduCombo] —— 模型×时长×分辨率合法组合硬校验
//（防超预期费用；host seconds 上限另在桥接层兜底）。
function validateCombo(model, duration, resolution, hasImages) {
  if (model === "vidu2.0" && !hasImages) {
    throw new Error("vidu2.0 does not support text-to-video");
  }
  if (model === "viduq1") {
    if (duration !== 5) throw new Error("viduq1 duration must be 5");
    return;
  }
  if (model === "vidu2.0") {
    if (duration === 4) {
      if (!["360p", "720p", "1080p"].includes(resolution)) {
        throw new Error("vidu2.0 duration 4 only allows resolution 360p, 720p, or 1080p");
      }
      return;
    }
    if (duration === 8) {
      if (resolution !== "720p") {
        throw new Error("vidu2.0 duration 8 only allows resolution 720p");
      }
      return;
    }
    throw new Error("vidu2.0 duration must be 4 or 8");
  }
  if (isQ2(model)) {
    if (!(duration >= 1 && duration <= 10)) {
      throw new Error("viduq2 duration must be between 1 and 10");
    }
    return;
  }
  // vidu1.5 等 [照抄 `seconds must be between 1 and 3600`]
  if (!(duration >= 1 && duration <= 3600)) {
    throw new Error("seconds must be between 1 and 3600");
  }
}

// 参考图 → data-URL 或原 URL（[照抄本仓 VideoInputRef::to_wire_string]）。
function toWireImage(ref) {
  if (!ref) return "";
  if (ref.url) return ref.url;
  if (ref.b64Json) return `data:${ref.mime || "image/png"};base64,${ref.b64Json}`;
  return "";
}

function pathFor(imageCount) {
  return imageCount === 0
    ? "/ent/v2/text2video"
    : imageCount === 1
      ? "/ent/v2/img2video"
      : imageCount === 2
        ? "/ent/v2/start-end2video"
        : "/ent/v2/reference2video";
}

// 提交：build 类单参 (ctx)。参考图张数决定端点；>2 张强制 viduq2
// [照抄 new-api buildSubmitRequest 的 reference_to_video 分支]。
export function buildSubmitRequest(ctx) {
  const model = ctx.model;
  const images = (ctx.request.inputReferences || [])
    .map(toWireImage)
    .filter((s) => s.length > 0);

  if (images.length > 2 && !model.includes("viduq2")) {
    throw new Error("vidu: reference-to-video (>2 images) requires a viduq2 model");
  }

  // 时长：整数解析（照抄 Rust i64 parse 语义），缺失/非法 → 模型默认。
  const raw = String(ctx.request.seconds ?? "").trim();
  const parsed = /^[-+]?[0-9]+$/.test(raw) ? Number(raw) : NaN;
  const duration = Number.isFinite(parsed) ? parsed : defaultDuration(model);

  const resolution = normalizeResolution(ctx.request.size ?? "", model);
  validateCombo(model, duration, resolution, images.length > 0);

  const body = {
    model,
    duration,
    resolution,
    movement_amplitude: "auto",
  };
  const prompt = String(ctx.request.prompt ?? "").trim();
  if (prompt) body.prompt = prompt;
  if (images.length) body.images = images;

  return {
    url: ctx.baseUrl + pathFor(images.length),
    method: "POST",
    headers: { Authorization: "Token " + ctx.apiKey },
    body,
  };
}

// 提交响应 → 任务句柄 [照抄 parseSubmitResponse：state=failed 报错、缺
// task_id 报错；state→status 照抄原生 status_from_wire，未知 → in_progress]。
const SUBMIT_STATE_STATUS = {
  created: "queued",
  queueing: "queued",
  processing: "in_progress",
  success: "completed",
};

export function parseSubmitResponse(ctx, response) {
  const body = response.body || {};
  const state = String(body.state || "");
  if (state === "failed") {
    throw new Error("vidu submit failed: " + String(body.err_code || ""));
  }
  const taskId = String(body.task_id ?? "");
  if (!taskId) throw new Error("vidu submit response has no task_id");
  return { taskId, status: SUBMIT_STATE_STATUS[state] ?? "in_progress" };
}

// 轮询：GET /ent/v2/tasks/{taskId}/creations。
export function buildQueryRequest(ctx) {
  return {
    url: ctx.baseUrl + "/ent/v2/tasks/" + ctx.taskId + "/creations",
    method: "GET",
    headers: { Authorization: "Token " + ctx.apiKey },
  };
}

// 轮询响应 → 任务快照 [照抄 parseTaskResult 状态映射 + err_code；
// unknown → in_progress（sweep 容忍，与原生 deviation 一致）]。
export function parseTaskResult(ctx, response) {
  const body = response.body || {};
  const state = String(body.state || "");
  if (state === "success") {
    const url = String(((body.creations || [])[0] || {}).url || "");
    return url ? { status: "completed", url } : { status: "completed" };
  }
  if (state === "failed") {
    return { status: "failed", error: String(body.err_code || "task failed") };
  }
  return { status: "in_progress" };
}
