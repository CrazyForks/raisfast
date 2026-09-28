// Alibaba Wan (通义万相) provider 扩展 —— 自原生
// crates/core/src/llm/providers/wan.rs 迁移（逻辑逐字段对照原生实现与其
// 测试断言；协议参照 new-api plugins/tasks/alibaba/plugin.js）。
//
// DashScope 异步协议：submit POST …/video-synthesis（X-DashScope-Async:
// enable），poll GET /api/v1/tasks/{id}；信封 {output:{task_id,task_status}}，
// 错误信封 {code,message}（code truthy 即错，照抄 new-api `if (body.code)`）。

export const meta = {
  key: "wan", // llm_channels.provider 的匹配键
  name: "Alibaba Wan (通义万相)",
  version: "1.0.0",
  contract: 1,
  protocols: ["video"],
  // 建议模型清单（UX 预填；权威源 = llm_models 目录）。
  models: [
    "wan3.0-video",
    "wan3.0-video-prime",
    "wan2.7-t2v",
    "wan2.7-i2v",
    "wan2.6-t2v",
    "wan2.6-i2v",
    "wan2.5-t2v-preview",
    "wan2.2-t2v-plus",
    "wanx2.1-t2v-plus",
  ],
  description: "通义万相文生视频/图生视频/首尾帧/参考生视频（DashScope 异步任务）",
  http: ["dashscope.aliyuncs.com/*"],
  timeout_ms: 30000,
};

// ── 模型付费矩阵 [照抄 new-api WAN_MODELS / 本仓 wan_profile!] ──────
// kind: all(统一端点+media) / t2v / size(老协议像素串) / image(img_url) /
// media(media 数组) / frames(首尾帧) / speech(口播，本仓不支持)。
// durations: null = 整数 2..max（kind all 另允许 -1 智能时长）。
const MODELS = {
  "wan3.0-video": { kind: "all", resolutions: ["480P", "720P", "1080P"], def: "1080P", max: 30 },
  "wan3.0-video-prime": { kind: "all", resolutions: ["480P", "720P", "1080P"], def: "1080P", max: 30 },
  "wan2.7-t2v": { kind: "t2v", resolutions: ["720P", "1080P"], def: "1080P", max: 15 },
  "wan2.7-i2v": { kind: "media", resolutions: ["720P", "1080P"], def: "1080P", max: 15 },
  "wan2.6-t2v": { kind: "size", resolutions: ["720P", "1080P"], def: "1080P", max: 15 },
  "wan2.6-t2v-us": { kind: "size", resolutions: ["720P", "1080P"], def: "1080P", durations: [5, 10, 15] },
  "wan2.6-i2v": { kind: "image", resolutions: ["720P", "1080P"], def: "1080P", max: 15 },
  "wan2.6-i2v-flash": { kind: "image", resolutions: ["720P", "1080P"], def: "1080P", max: 15 },
  "wan2.6-i2v-us": { kind: "image", resolutions: ["720P", "1080P"], def: "1080P", durations: [5, 10, 15] },
  "wan2.5-t2v-preview": { kind: "size", resolutions: ["480P", "720P", "1080P"], def: "1080P", durations: [5, 10] },
  "wan2.5-i2v-preview": { kind: "image", resolutions: ["480P", "720P", "1080P"], def: "1080P", durations: [5, 10] },
  "wan2.2-t2v-plus": { kind: "size", resolutions: ["480P", "1080P"], def: "1080P", durations: [5] },
  "wan2.2-i2v-flash": { kind: "image", resolutions: ["480P", "720P", "1080P"], def: "720P", durations: [5] },
  "wan2.2-i2v-plus": { kind: "image", resolutions: ["480P", "1080P"], def: "1080P", durations: [5] },
  "wan2.2-kf2v-flash": { kind: "frames", resolutions: ["480P", "720P", "1080P"], def: "720P", durations: [5] },
  "wan2.2-s2v": { kind: "speech", resolutions: ["480P", "720P"], def: "480P", durations: [20] },
  "wanx2.1-t2v-plus": { kind: "size", resolutions: ["720P"], def: "720P", durations: [5] },
  "wanx2.1-t2v-turbo": { kind: "size", resolutions: ["480P", "720P"], def: "720P", durations: [5] },
  "wanx2.1-i2v-plus": { kind: "image", resolutions: ["720P"], def: "720P", durations: [5] },
  "wanx2.1-i2v-turbo": { kind: "image", resolutions: ["480P", "720P"], def: "720P", durations: [3, 4, 5] },
};

// [照抄 new-api modelKey] —— 去掉 -YYYY-MM-DD 日期后缀，wan2.1- → wanx2.1-。
function modelKey(model) {
  const key = String(model ?? "").trim();
  const isDate = (s) =>
    s.length === 10 && s[4] === "-" && s[7] === "-" && /^[0-9]{4}-[0-9]{2}-[0-9]{2}$/.test(s);
  let base = key;
  if (key.length >= 11) {
    const head = key.slice(0, -11);
    const tail = key.slice(-11);
    if (tail.startsWith("-") && isDate(tail.slice(1)) && head.length > 0) base = head;
  }
  if (base.startsWith("wan2.1-")) return "wanx2.1-" + base.slice(7);
  return base;
}

function profile(model) {
  return MODELS[modelKey(model)] || null;
}

// [照抄 new-api LEGACY_SIZES] —— kind=size 模型的像素串表。
function legacyPixelSize(resolution, ratio) {
  const table = {
    "720P": { "16:9": "1280*720", "9:16": "720*1280", "1:1": "960*960", "4:3": "1104*832", "3:4": "832*1104" },
    "1080P": { "16:9": "1920*1080", "9:16": "1080*1920", "1:1": "1440*1440", "4:3": "1632*1248", "3:4": "1248*1632" },
    "480P": { "16:9": "832*480", "9:16": "480*832", "1:1": "672*672", "4:3": "768*576", "3:4": "576*768" },
  };
  return (table[resolution] || {})[ratio] || null;
}

// [自造-适配] 开放 WxH/W*H → 最近档位：最长边定 resolution，宽高比定 ratio。
function sizeToTiers(size) {
  const normalized = String(size ?? "").trim().replace(/x/i, "*");
  const parts = normalized.split("*");
  if (parts.length !== 2) return null;
  const w = Number(parts[0].trim());
  const h = Number(parts[1].trim());
  if (!(w > 0) || !(h > 0)) return null;
  const max = Math.max(w, h);
  const resolution = max >= 1620 ? "1080P" : max >= 1000 ? "720P" : "480P";
  const ratio = w / h;
  const candidates = [
    [16 / 9, "16:9"],
    [9 / 16, "9:16"],
    [1, "1:1"],
    [4 / 3, "4:3"],
    [3 / 4, "3:4"],
  ];
  let best = candidates[0];
  for (const c of candidates) {
    if (Math.abs(c[0] - ratio) < Math.abs(best[0] - ratio)) best = c;
  }
  return [resolution, best[1]];
}

// 参考图 → data-URL 或原 URL（[照抄本仓 VideoInputRef::to_wire_string]）。
function toWireImage(ref) {
  if (!ref) return "";
  if (ref.url) return ref.url;
  if (ref.b64Json) return `data:${ref.mime || "image/png"};base64,${ref.b64Json}`;
  return "";
}

// 提交：build 类 (ctx)。模型 kind 决定端点与 input 形状。
export function buildSubmitRequest(ctx) {
  const p = profile(ctx.model);
  if (!p) throw new Error(`wan: unsupported model ${JSON.stringify(ctx.model)}`);
  if (p.kind === "speech") {
    throw new Error("wan: wan2.2-s2v needs an audio input surface this provider does not expose");
  }

  const refs = (ctx.request.inputReferences || []).map(toWireImage).filter((s) => s.length > 0);

  const input = {};
  const prompt = String(ctx.request.prompt ?? "").trim();
  if (prompt) input.prompt = prompt;
  if (p.kind === "image") {
    if (refs.length !== 1) {
      throw new Error(`wan: model ${ctx.model} (kind image) does not take ${refs.length} reference image(s)`);
    }
    input.img_url = refs[0];
  } else if (p.kind === "media") {
    if (refs.length < 1 || refs.length > 2) {
      throw new Error(`wan: model ${ctx.model} (kind media) does not take ${refs.length} reference image(s)`);
    }
    input.media = [{ type: "first_frame", url: refs[0] }];
    if (refs.length === 2) input.media.push({ type: "last_frame", url: refs[1] });
  } else if (p.kind === "frames") {
    if (refs.length < 1 || refs.length > 2) {
      throw new Error(`wan: model ${ctx.model} (kind frames) does not take ${refs.length} reference image(s)`);
    }
    input.first_frame_url = refs[0];
    if (refs.length === 2) input.last_frame_url = refs[1];
  } else if (refs.length > 0) {
    if (p.kind === "all") {
      input.media = refs.map((url) => ({ type: "reference_image", url }));
    } else {
      throw new Error(`wan: model ${ctx.model} does not take ${refs.length} reference image(s)`);
    }
  }

  // parameters [照抄 new-api convert：prompt_extend 默认开；kind 决定
  // size/resolution/ratio 组装]。
  const parameters = { prompt_extend: true };
  const tiers = sizeToTiers(ctx.request.size ?? "");
  if (tiers) {
    const [resolution, ratio] = tiers;
    if (p.kind === "size") {
      const pixel = legacyPixelSize(resolution, ratio);
      if (!pixel) throw new Error(`wan: unsupported ratio for ${resolution}: ${ratio}`);
      parameters.size = pixel;
    } else if (p.kind === "t2v" || p.kind === "all") {
      parameters.resolution = resolution;
      parameters.ratio = ratio;
    } else {
      // 图生视频类：宽高比不可配，仅 resolution。
      parameters.resolution = resolution;
    }
  } else {
    parameters.resolution = p.def;
    if (p.kind === "t2v" || p.kind === "size") parameters.ratio = "16:9";
    else if (p.kind === "all") parameters.ratio = "adaptive";
  }

  // 时长：缺失 → 模型默认 5；非数字 → 报错（照抄 Rust parse 语义）；
  // 负数（-1 智能时长）合法，交给下方档位校验 [照抄 convert]。
  const raw = String(ctx.request.seconds ?? "").trim();
  let duration = 5;
  if (raw !== "") {
    if (/^[-+]?[0-9]+$/.test(raw)) duration = Number(raw);
    else throw new Error(`wan: duration must be a number, got ${JSON.stringify(raw)}`);
  }
  parameters.duration = duration;

  // 渠道 paramOverride 浅合并（seed/prompt_extend/watermark…）。
  if (ctx.paramOverride && typeof ctx.paramOverride === "object") {
    Object.assign(parameters, ctx.paramOverride);
  }

  // 付费档位硬校验（合并后仍校验——防超预期费用，§5.4）。
  if (parameters.resolution && !p.resolutions.includes(parameters.resolution)) {
    throw new Error(`wan: ${ctx.model} resolution must be one of ${p.resolutions.join(", ")}`);
  }
  const dur = parameters.duration;
  if (p.durations) {
    if (!p.durations.includes(dur)) {
      throw new Error(`wan: ${ctx.model} duration must be one of ${p.durations.join(", ")}`);
    }
  } else {
    const smartOk = p.kind === "all" && dur === -1;
    if (!smartOk && !(dur >= 2 && dur <= p.max)) {
      throw new Error(
        `wan: ${ctx.model} duration must be ${p.kind === "all" ? "-1 or " : ""}an integer between 2 and ${p.max}`,
      );
    }
  }

  const service = p.kind === "image" || p.kind === "frames" ? "image2video" : "video-generation";
  return {
    url: ctx.baseUrl + `/api/v1/services/aigc/${service}/video-synthesis`,
    method: "POST",
    headers: {
      Authorization: "Bearer " + ctx.apiKey,
      "X-DashScope-Async": "enable",
    },
    body: { model: ctx.model, input, parameters },
  };
}

// 提交响应 → 任务句柄。
// 信封错误 [照抄 new-api `if (body.code) throw` —— code 可为字符串或数字]。
export function parseSubmitResponse(ctx, response) {
  const body = response.body || {};
  const code = body.code;
  if (code !== undefined && code !== null && code !== "" && code !== "0" && code !== 0) {
    throw new Error("dashscope code " + code + ": " + (body.message || ""));
  }
  const output = body.output || {};
  const taskId = String(output.task_id ?? "");
  if (!taskId) throw new Error("wan: envelope without output.task_id");
  const state = String(output.task_status || "");
  const status =
    state === "PENDING" ? "queued" : state === "SUCCEEDED" ? "completed" : "in_progress";
  return { taskId, status };
}

// 轮询：GET /api/v1/tasks/{taskId}。
export function buildQueryRequest(ctx) {
  return {
    url: ctx.baseUrl + "/api/v1/tasks/" + ctx.taskId,
    method: "GET",
    headers: { Authorization: "Bearer " + ctx.apiKey },
  };
}

// 轮询响应 → 任务快照 [照抄 parseTaskResult：FAILED/CANCELED/UNKNOWN →
// failed（reason 链）；其余 in_progress（sweep 容忍）]。
export function parseTaskResult(ctx, response) {
  const body = response.body || {};
  const output = body.output || {};
  const state = String(output.task_status || "");
  const reason =
    body.message ||
    (output.message ? `task failed, code: ${output.code || ""} , message: ${output.message}` : "") ||
    "task failed";
  if (state === "SUCCEEDED") {
    const url = output.video_url || (output.results || {}).video_url || "";
    return url ? { status: "completed", url } : { status: "completed" };
  }
  if (state === "PENDING") return { status: "queued" };
  if (state === "FAILED" || state === "CANCELED" || state === "UNKNOWN") {
    return { status: "failed", error: reason };
  }
  return { status: "in_progress" };
}
