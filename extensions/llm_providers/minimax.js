// MiniMax (海螺) provider 扩展 —— 自原生
// crates/core/src/llm/providers/minimax.rs 迁移（逻辑逐字段对照原生实现
// 与其测试断言；协议参照 new-api plugins/tasks/hailuo/plugin.js + MiniMax
// 公开 API 文档）。
//
// 四模态全覆盖（契约 v1 多模态扩展）：chat（chatcompletion_v2，非流式）+
// speech（t2a_v2，hex 音频）+ music（music_generation，hex 音频）+ video
// （V1/V2 双代协议）。
//
// key 格式 `api_key:group_id`——speech/music 强制 GroupId；chat/video 仅用
// api_key。

export const meta = {
  key: "minimax", // llm_channels.provider 的匹配键
  name: "MiniMax (海螺)",
  version: "1.1.0",
  contract: 1,
  protocols: ["chat", "speech", "music", "video"],
  // 建议模型清单（UX 预填；权威源 = llm_models 目录）。
  models: [
    "MiniMax-Text-01",
    "MiniMax-H3",
    "MiniMax-H3-Max",
    "MiniMax-Hailuo-02",
    "MiniMax-Hailuo-2.3",
    "speech-2.8-hd",
    "speech-02-hd",
    "music-01",
  ],
  description:
    "海螺 Chat/语音/音乐/视频。视频 V2（/v2，H3 与 H3-Max 共用 content[] 协议）；V1 仅存量 Hailuo-02/2.3；TTS/音乐需 GroupId",
  // global（api.minimax.io）与 CN（api.minimaxi.com）双 host——
  // TTS speech-2.8-hd 走 global [照抄 MPT voice.py MINIMAX_TTS_GLOBAL/CN_URL]。
  http: ["api.minimaxi.com/*", "api.minimax.io/*"],
  timeout_ms: 30000,
};

const H3_MODEL = "MiniMax-H3";
const H3_MAX_MODEL = "MiniMax-H3-Max";
// V2 协议模型（/v2/video_generation，content[] 多模态输入）[照抄官方 OpenAPI
// video-generation-v2-create：H3 768P/2K、4-15s；H3-Max 快速版 480P/768P
// （不支持 2K）、5-15s，extra.prompt_expansion_mode 可选]。
const V2_MODELS = [H3_MODEL, H3_MAX_MODEL];
const H3_DEFAULT_DURATION = 5;
const H3_RATIOS = ["adaptive", "21:9", "16:9", "4:3", "1:1", "3:4", "9:16"];

function isV2(model) {
  return V2_MODELS.includes(model);
}

function v2MinDuration(model) {
  return model === H3_MAX_MODEL ? 5 : 4;
}

function isModernHailuo(model) {
  return (
    model === "MiniMax-Hailuo-2.3" ||
    model === "MiniMax-Hailuo-2.3-Fast" ||
    model === "MiniMax-Hailuo-02"
  );
}

// key 格式 `api_key:group_id`（group 可缺省）。
function splitKey(apiKey) {
  const raw = String(apiKey ?? "");
  const i = raw.indexOf(":");
  if (i <= 0) return [raw.trim(), null];
  return [raw.slice(0, i).trim(), raw.slice(i + 1).trim() || null];
}

// ── V2 参数 helpers [照抄 plugin h3Duration/h3Resolution/h3Ratio + 官方
// OpenAPI 模型差异：H3-Max 时长 5-15、分辨率 480P/768P 无 2K] ─────────

function h3Duration(seconds, model) {
  const raw = String(seconds ?? "").trim();
  if (raw === "") return H3_DEFAULT_DURATION;
  const n = Number(raw);
  const min = v2MinDuration(model);
  if (!Number.isInteger(n) || n < min || n > 15) {
    throw new Error(
      `${model} duration must be an integer between ${min} and 15 seconds`,
    );
  }
  return n;
}

function h3VideoResolution(ctx) {
  const model = ctx.model;
  const raw =
    String(ctx.request.size ?? "").trim().toUpperCase() ||
    String((ctx.paramOverride || {}).resolution ?? "").trim().toUpperCase();
  if (raw === "") return "768P";
  if (raw.includes("2K")) {
    if (model === H3_MAX_MODEL) {
      throw new Error(`${H3_MAX_MODEL} does not support 2K (480P/768P only)`);
    }
    return "2K";
  }
  if (raw.includes("768")) return "768P";
  if (raw.includes("480")) {
    if (model !== H3_MAX_MODEL) {
      throw new Error(`${model} does not support 480P (768P/2K only)`);
    }
    return "480P";
  }
  throw new Error(`${model} resolution must be 480P/768P${model === H3_MAX_MODEL ? "" : "/2K"}`);
}

function h3Ratio(paramOverride, hasVisual) {
  const ratio = String((paramOverride || {}).ratio ?? "").trim() || (hasVisual ? "adaptive" : "16:9");
  if (!H3_RATIOS.includes(ratio)) {
    throw new Error(`${H3_MODEL} ratio must be one of ${H3_RATIOS.join(", ")}`);
  }
  if (ratio === "adaptive" && !hasVisual) {
    throw new Error(`${H3_MODEL} ratio adaptive requires an image or video input`);
  }
  return ratio;
}

// ── V1 helpers [照抄 plugin outboundDuration/resolutionFor] ─────────

function v1Duration(seconds) {
  const n = Number(String(seconds ?? "").trim());
  return Number.isInteger(n) && n > 0 ? n : 6;
}

function v1DefaultResolution(model) {
  return ["MiniMax-Hailuo-2.3", "MiniMax-Hailuo-2.3-Fast", "MiniMax-Hailuo-02"].includes(model)
    ? "768P"
    : "720P";
}

function v1Resolution(size, paramOverride, model) {
  const raw =
    String(size ?? "").trim() || String((paramOverride || {}).resolution ?? "").trim();
  if (raw === "") return v1DefaultResolution(model);
  if (raw.includes("1080")) return "1080P";
  if (raw.includes("768")) return "768P";
  if (raw.includes("720")) return isModernHailuo(model) ? "768P" : "720P";
  if (raw.includes("512")) return "512P";
  return v1DefaultResolution(model);
}

// ── 信封 [照抄原生 check_envelope：顶层 error 与 base_resp 双层] ──────
// 偏差标注：原生映射 Http{http_code}（可 failover 分类），JS throw →
// Config（渠道级）——与 kling 信封偏差同款，MVP 接受。
function checkEnvelope(body) {
  const err = body.error;
  if (err && typeof err === "object") {
    const message = String(err.message || "");
    if (message) {
      const code = Number(err.http_code ?? err.code ?? 0) || 500;
      throw new Error(`minimax http ${code}: ${message}`);
    }
  }
  const code = Number((body.base_resp || {}).status_code ?? 0);
  if (code !== 0) {
    throw new Error(`minimax code ${code}: ${(body.base_resp || {}).status_msg || ""}`);
  }
}

// [照抄原生 status_from_wire：submit 顶层 status / H3 query 包在 task 里；
//  V1 Success/Fail/Preparing/Queueing/Processing；未知 → in_progress]
function videoStatus(parsed, h3) {
  if (h3) {
    const status = String(
      (parsed.task && parsed.task.status) || parsed.status || "",
    );
    if (status === "succeeded") return "completed";
    if (status === "failed" || status === "cancelled") return "failed";
    if (status === "queued") return "queued";
    return "in_progress";
  }
  const status = String(parsed.status || "");
  if (status === "Success") return "completed";
  if (status === "Fail") return "failed";
  return "in_progress";
}

// ── chat（chatcompletion_v2，OpenAI 同形线协议）─────────────────────

export function buildChatRequest(ctx) {
  const messages = (ctx.request.messages || []).map((m) => {
    if (m.images && m.images.length > 0) {
      const parts = [];
      if (m.content) parts.push({ type: "text", text: m.content });
      for (const u of m.images) parts.push({ type: "image_url", image_url: { url: u } });
      const out = { role: m.role, content: parts };
      if (m.toolCalls) out.tool_calls = m.toolCalls;
      return out;
    }
    const out = { role: m.role, content: m.content ?? "" };
    if (m.toolCalls) out.tool_calls = m.toolCalls;
    return out;
  });
  const body = { model: ctx.model, messages };
  if (ctx.request.temperature !== null && ctx.request.temperature !== undefined) {
    body.temperature = ctx.request.temperature;
  }
  if (ctx.request.maxTokens !== null && ctx.request.maxTokens !== undefined) {
    body.max_tokens = ctx.request.maxTokens;
  }
  if (ctx.request.stop && ctx.request.stop.length > 0) body.stop = ctx.request.stop;
  if (ctx.request.tools && ctx.request.tools.length > 0) {
    body.tools = ctx.request.tools.map((t) => ({
      type: "function",
      function: { name: t.name, description: t.description, parameters: t.parameters },
    }));
  }
  if (ctx.paramOverride && typeof ctx.paramOverride === "object") {
    Object.assign(body, ctx.paramOverride);
  }
  return {
    url: ctx.baseUrl + "/v1/text/chatcompletion_v2",
    method: "POST",
    headers: { Authorization: "Bearer " + splitKey(ctx.apiKey)[0] },
    body,
  };
}

// [照抄原生 parse_chat_response：choices[0].message + usage.prompt/completion]
export function parseChatResponse(ctx, response) {
  const body = response.body || {};
  checkEnvelope(body);
  const choice = (body.choices || [])[0];
  if (!choice) throw new Error("minimax: chat response without choices");
  const message = choice.message || {};
  const toolCalls = (message.tool_calls || []).map((c) => ({
    id: String(c.id ?? ""),
    name: String((c.function || {}).name ?? ""),
    arguments: String((c.function || {}).arguments ?? ""),
  }));
  const usage = body.usage
    ? {
        inputTokens: body.usage.prompt_tokens ?? null,
        outputTokens: body.usage.completion_tokens ?? null,
      }
    : null;
  return {
    text: message.content ?? null,
    toolCalls,
    usage: usage && (usage.inputTokens !== null || usage.outputTokens !== null) ? usage : null,
  };
}

// ── speech / music（GroupId 强制；hex 音频透传宿主解码）─────────────

function audioEnvelope(ctx, response) {
  const body = response.body || {};
  checkEnvelope(body);
  const hex = String(((body.data || {}).audio) ?? "");
  const bytes = hex.trim();
  if (!bytes) throw new Error("minimax: empty audio");
  return bytes;
}

export function buildSpeechRequest(ctx) {
  const [key, group] = splitKey(ctx.apiKey);
  // GroupId [自造-放宽 2026-09]：CN 老 API 强制；全球新版（api.minimax.io，
  // 官方 OpenAPI）已无此参数——缺省直通，给了才追加 query。
  const body = {
    model: ctx.model,
    text: ctx.request.text,
    voice_setting: { voice_id: ctx.request.voice },
    audio_setting: { format: "mp3", sample_rate: 32000, bitrate: 128000, channel: 1 },
  };
  if (ctx.paramOverride && typeof ctx.paramOverride === "object") {
    Object.assign(body, ctx.paramOverride);
  }
  return {
    url: ctx.baseUrl + "/v1/t2a_v2" + (group ? "?GroupId=" + encodeURIComponent(group) : ""),
    method: "POST",
    headers: { Authorization: "Bearer " + key },
    body,
  };
}

export function parseSpeechResponse(ctx, response) {
  return { audioHex: audioEnvelope(ctx, response) };
}

export function buildMusicRequest(ctx) {
  const [key, group] = splitKey(ctx.apiKey);
  // GroupId [自造-放宽]：同 speech——全球新版缺省直通（music API 官方已停
  // 售新用户，存量兼容保留）。
  const body = {
    model: ctx.model,
    prompt: ctx.request.prompt,
    audio_setting: { format: "mp3", sample_rate: 32000, bitrate: 128000, channel: 1 },
  };
  const lyrics = String(ctx.request.lyrics ?? "").trim();
  if (lyrics) body.lyrics = lyrics;
  if (ctx.paramOverride && typeof ctx.paramOverride === "object") {
    Object.assign(body, ctx.paramOverride);
  }
  return {
    url: ctx.baseUrl + "/v1/music_generation" + (group ? "?GroupId=" + encodeURIComponent(group) : ""),
    method: "POST",
    headers: { Authorization: "Bearer " + key },
    body,
  };
}

export function parseMusicResponse(ctx, response) {
  return { audioHex: audioEnvelope(ctx, response) };
}

// ── video（V1/V2 双代协议）──────────────────────────────────────────

function h3VideoRatio(ctx, hasVisual) {
  const ratio =
    String((ctx.paramOverride || {}).ratio ?? "").trim() || (hasVisual ? "adaptive" : "16:9");
  if (!H3_RATIOS.includes(ratio)) {
    throw new Error(`${ctx.model} ratio must be one of ${H3_RATIOS.join(", ")}`);
  }
  if (ratio === "adaptive" && !hasVisual) {
    throw new Error(`${ctx.model} ratio adaptive requires an image or video input`);
  }
  return ratio;
}

function toWireImage(ref) {
  if (!ref) return "";
  if (ref.url) return ref.url;
  if (ref.b64Json) return `data:${ref.mime || "image/png"};base64,${ref.b64Json}`;
  return "";
}

// 提交：H3 → /v2/video_generation（multimodal content，参考图 i2v）；
// 其余 → /v1/video_generation（flat fields，首参考图 → first_frame_image）。
// 渠道 paramOverride 浅合并（跳过 resolution/ratio/duration——H3 由校验产出）。
export function buildSubmitRequest(ctx) {
  const prompt = String(ctx.request.prompt ?? "");
  if (!prompt.trim()) throw new Error("minimax: prompt must not be empty");

  const refs = (ctx.request.inputReferences || []).map(toWireImage).filter((s) => s.length > 0);
  const owned = ["resolution", "ratio", "duration"];
  const overrides = ctx.paramOverride && typeof ctx.paramOverride === "object" ? ctx.paramOverride : {};
  let url;
  let body;
  if (isV2(ctx.model)) {
    const duration = h3Duration(ctx.request.seconds, ctx.model);
    const resolution = h3VideoResolution(ctx);
    const content = [{ type: "text", text: prompt }];
    for (const wire of refs) {
      content.push({ type: "image_url", image_url: { url: wire }, role: "first_frame" });
    }
    const ratio = h3VideoRatio(ctx, refs.length > 0);
    body = { model: ctx.model, content, resolution, duration, ratio };
    for (const [k, v] of Object.entries(overrides)) {
      if (!owned.includes(k)) body[k] = v;
    }
    url = ctx.baseUrl + "/v2/video_generation";
  } else {
    body = {
      model: ctx.model,
      prompt,
      duration: v1Duration(ctx.request.seconds),
      resolution: v1Resolution(ctx.request.size, overrides, ctx.model),
    };
    for (const [k, v] of Object.entries(overrides)) {
      if (!owned.includes(k)) body[k] = v;
    }
    if (refs.length > 0) body.first_frame_image = refs[0];
    url = ctx.baseUrl + "/v1/video_generation";
  }
  if (ctx.request.callbackUrl) body.callback_url = ctx.request.callbackUrl;
  return { url, method: "POST", headers: { Authorization: "Bearer " + splitKey(ctx.apiKey)[0] }, body };
}

// 提交响应 → 任务句柄（task_id 必需；status 按代际词汇映射）。
export function parseSubmitResponse(ctx, response) {
  const body = response.body || {};
  checkEnvelope(body);
  const taskId = String(body.task_id ?? "");
  if (!taskId) throw new Error("minimax: submit response without task_id");
  return { taskId, status: videoStatus(body, isV2(ctx.model)) };
}

// 轮询：H3 GET /v2/query/video_generation/{id}；V1 GET /v1/query?task_id=。
export function buildQueryRequest(ctx) {
  if (isV2(ctx.model)) {
    return {
      url: ctx.baseUrl + "/v2/query/video_generation/" + encodeURIComponent(ctx.taskId),
      method: "GET",
      headers: { Authorization: "Bearer " + splitKey(ctx.apiKey)[0] },
    };
  }
  return {
    url: ctx.baseUrl + "/v1/query/video_generation?task_id=" + encodeURIComponent(ctx.taskId),
    method: "GET",
    headers: { Authorization: "Bearer " + splitKey(ctx.apiKey)[0] },
  };
}

// 轮询响应 → 任务快照 + V1 下载信息（file_id → 鉴权下载 URL + headers，
// 宿主按 downloadHeaders 下载；H3 走公网 CDN 免鉴权）。
export function parseTaskResult(ctx, response) {
  const body = response.body || {};
  checkEnvelope(body);
  const h3 = isV2(ctx.model);
  const status = videoStatus(body, h3);
  const failedReason = () => {
    if (h3) {
      const msg =
        ((body.task || {}).error || {}).message ||
        ((body.base_resp || {}).status_msg || "");
      return msg || "task failed";
    }
    return String((body.base_resp || {}).status_msg || "task failed") || "task failed";
  };
  const out = { status };
  if (status === "failed") out.error = failedReason();
  if (status === "completed") {
    if (h3) {
      const url = String((((body.task || {}).content || {}).url) || "");
      if (url) out.url = url;
    } else {
      const fileId = String(body.file_id ?? "");
      if (fileId) {
        out.url = ctx.baseUrl + "/v1/files/download?file_id=" + encodeURIComponent(fileId);
        out.downloadHeaders = { Authorization: "Bearer " + ctx.apiKey };
      }
    }
  }
  return out;
}
