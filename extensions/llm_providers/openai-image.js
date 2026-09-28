// OpenAI 兼容文生图 provider 扩展 —— 自原生
// third/MoneyPrinterTurbo/app/services/material.py（Issue #1274）迁移，
// image 协议的首个消费者。
//
// 协议：POST /images/generations，body {model, prompt, n, size}；
// 响应 {data: [{b64_json | url}]}。
//
// 适配说明 [照抄 MPT material.py]：
// - Authorization 可空——完全本地的 ComfyUI/SD 网关通常不需要鉴权；
// - 同步生成可能需要数十秒（meta.timeout_ms 给足 300s）；
// - 自定义 host（本地网关/中转）需编辑 meta.http 白名单。

// 参考图 → data-URL 或原 URL（[照抄本仓 VideoInputRef::to_wire_string]）。
function toWireImage(ref) {
  if (!ref) return undefined;
  if (ref.url) return ref.url;
  if (ref.b64Json) return `data:${ref.mime || "image/png"};base64,${ref.b64Json}`;
  return undefined;
}

export const meta = {
  key: "openai-image", // llm_channels.provider 的匹配键
  name: "OpenAI 兼容文生图",
  version: "1.0.0",
  contract: 1,
  protocols: ["image"],
  models: ["gpt-image-1", "dall-e-3"],
  description: "OpenAI 兼容 /images/generations 文生图（支持本地 ComfyUI/SD 网关，key 可空）",
  http: ["api.openai.com/*"],
  timeout_ms: 300000,
};

// 生成：POST /images/generations。apiKey 为空时不带 Authorization 头
// [照抄 MPT：本地 ComfyUI/SD 网关通常不需要鉴权]。
// 参考图（图生图/角色一致性）：首个 ref 走 OpenAI wire `image` 参数
// [照抄原生 agent OpenAiCompatProvider.generate_image——聚合器映射到
// 厂商原生字段（Seedream refs、Kling edit image）]。
export function buildImageRequest(ctx) {
  const prompt = String(ctx.request.prompt ?? "");
  if (!prompt.trim()) throw new Error("openai-image: prompt must not be empty");

  const body = {
    model: ctx.model,
    prompt,
    n: Number(ctx.request.n ?? 1) || 1,
  };
  if (ctx.request.size) body.size = String(ctx.request.size);
  const ref = toWireImage((ctx.request.inputReferences || [])[0]);
  if (ref) body.image = ref;

  const headers = { "Content-Type": "application/json" };
  if (ctx.apiKey) headers.Authorization = "Bearer " + ctx.apiKey;

  return {
    url: ctx.baseUrl + "/images/generations",
    method: "POST",
    headers,
    body,
  };
}

// 响应解析：data[].{b64_json | url} → {images: [{b64Json} | {url}]}。
// [照抄 MPT _parse_openai_image_response：url 需 http(s) 前缀；
//  b64_json 由宿主侧解码为字节——扩展不处理原始字节。]
export function parseImageResponse(ctx, response) {
  const body = response.body || {};
  const data = body.data;
  if (!Array.isArray(data) || data.length === 0) {
    throw new Error("openai-image: response has no data array");
  }
  const images = [];
  for (const entry of data) {
    if (!entry || typeof entry !== "object") continue;
    if (typeof entry.b64_json === "string" && entry.b64_json.length > 0) {
      images.push({ b64Json: entry.b64_json });
      continue;
    }
    if (typeof entry.url === "string" && entry.url.startsWith("http")) {
      images.push({ url: entry.url });
    }
  }
  if (images.length === 0) {
    throw new Error("openai-image: image response has neither url nor b64_json");
  }
  return { images };
}
