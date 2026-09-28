// Seedance 扩展契约测试 —— 断言 1:1 移植自原生 crates/core/src/llm/providers/
// seedance.rs 的 wiremock 测试（迁移行为防漂移，provider-plugins.md §9）。
//
// 运行：just test-llm-providers

import { test } from "node:test";
import assert from "node:assert/strict";
import { meta, buildSubmitRequest, parseSubmitResponse, buildQueryRequest, parseTaskResult } from "../seedance.js";

const CTX = (overrides = {}, paramOverride = {}) => ({
  provider: "seedance",
  baseUrl: "https://api.test",
  apiKey: "ark-key",
  model: "doubao-seedance-1-0-pro",
  paramOverride,
  request: {
    prompt: "夜景城市",
    seconds: null,
    size: null,
    inputReferences: [],
    callbackUrl: null,
  },
  taskId: null,
  ...overrides,
});

const ref = (name) => ({ url: `https://cdn.example.com/${name}.png` });

// ── 提交构建 ─────────────────────────────────────────────────────

test("submit posts content array with bearer and defaults", () => {
  const spec = buildSubmitRequest(CTX({ request: { prompt: "夜景城市", seconds: "5", size: "1080x1920" } }));
  assert.equal(spec.method, "POST");
  assert.equal(spec.url, "https://api.test/contents/generations/tasks");
  assert.equal(spec.headers.Authorization, "Bearer ark-key");
  assert.equal(spec.body.model, "doubao-seedance-1-0-pro");
  assert.equal(spec.body.content[0].type, "text");
  assert.equal(spec.body.content[0].text, "夜景城市");
  assert.equal(spec.body.ratio, "9:16", "1080x1920 → 9:16（MPT 默认竖屏）");
  assert.equal(spec.body.duration, 5);
  assert.equal(spec.body.resolution, "1080p");
  assert.equal(spec.body.watermark, false);
});

test("duration clamped to 2..12", () => {
  const high = buildSubmitRequest(CTX({ request: { prompt: "x", seconds: "60" } }));
  assert.equal(high.body.duration, 12, "clamp to MAX");
  const low = buildSubmitRequest(CTX({ request: { prompt: "x", seconds: "1" } }));
  assert.equal(low.body.duration, 2, "clamp to MIN");
  const invalid = buildSubmitRequest(CTX({ request: { prompt: "x", seconds: "abc" } }));
  assert.equal(invalid.body.duration, 2, "非法回落 MIN（照抄 unwrap_or(MIN)）");
});

test("empty prompt rejected before submit", () => {
  assert.throws(() => buildSubmitRequest(CTX({ request: { prompt: "   " } })), /prompt must not be empty/);
});

test("invalid resolution override is config error not silent", () => {
  assert.throws(
    () => buildSubmitRequest(CTX({}, { resolution: "4K" })),
    /unsupported seedance resolution "4k"/,
  );
});

test("resolution override accepted (validated enum)", () => {
  const spec = buildSubmitRequest(CTX({}, { resolution: "720P" }));
  assert.equal(spec.body.resolution, "720p", "归一小写");
  // skip-resolution 分支：override 的原始大小写键不再覆盖
  assert.ok(!("Resolution" in spec.body));
});

test("reference image becomes first_frame content entry", () => {
  const spec = buildSubmitRequest(
    CTX({ request: { prompt: "同款镜头", inputReferences: [ref("a")] } }),
  );
  assert.equal(spec.body.content[1].type, "image_url");
  assert.equal(spec.body.content[1].image_url.url, "https://cdn.example.com/a.png");
  assert.equal(spec.body.content[1].role, "first_frame");
});

test("b64 reference becomes data-URL first frame", () => {
  const spec = buildSubmitRequest(
    CTX({ request: { prompt: "x", inputReferences: [{ b64Json: "QUJD", mime: "image/jpeg" }] } }),
  );
  assert.equal(spec.body.content[1].image_url.url, "data:image/jpeg;base64,QUJD");
});

test("param_override merges other keys into body", () => {
  const spec = buildSubmitRequest(CTX({}, { resolution: "720p", callback_url: "https://hook.test/cb", camera_fixed: true }));
  assert.equal(spec.body.resolution, "720p");
  assert.equal(spec.body.callback_url, "https://hook.test/cb");
  assert.equal(spec.body.camera_fixed, true);
});

// ── 提交响应解析 ─────────────────────────────────────────────────

test("submit parses id and queued state", () => {
  const out = parseSubmitResponse(CTX(), {
    status: 200,
    body: { id: "cgt-1", status: "queued" },
  });
  assert.equal(out.taskId, "cgt-1");
  assert.equal(out.status, "queued");
});

test("submit missing id is an error", () => {
  assert.throws(() => parseSubmitResponse(CTX(), { status: 200, body: { status: "queued" } }), /without id/);
});

// ── 轮询 ─────────────────────────────────────────────────────────

test("buildQueryRequest polls task endpoint with bearer", () => {
  const spec = buildQueryRequest(CTX({ taskId: "cgt-9" }));
  assert.equal(spec.method, "GET");
  assert.equal(spec.url, "https://api.test/contents/generations/tasks/cgt-9");
  assert.equal(spec.headers.Authorization, "Bearer ark-key");
});

test("parseTaskResult maps succeeded with content.video_url", () => {
  const out = parseTaskResult(CTX(), {
    status: 200,
    body: { id: "cgt-9", status: "succeeded", content: { video_url: "https://oss.example.com/out.mp4" } },
  });
  assert.equal(out.status, "completed");
  assert.equal(out.url, "https://oss.example.com/out.mp4");
});

test("parseTaskResult maps terminal failures and tolerates unknown", () => {
  for (const state of ["failed", "cancelled", "canceled", "expired"]) {
    const out = parseTaskResult(CTX(), { status: 200, body: { status: state, error: "boom" } });
    assert.equal(out.status, "failed");
    assert.equal(out.error, "boom");
  }
  assert.equal(parseTaskResult(CTX(), { status: 200, body: { status: "running" } }).status, "in_progress");
  assert.equal(parseTaskResult(CTX(), { status: 200, body: { status: "queued" } }).status, "queued");
  assert.equal(parseTaskResult(CTX(), { status: 200, body: {} }).status, "in_progress", "未知状态 → in_progress");
});

// ── meta ─────────────────────────────────────────────────────────

test("meta declares video contract v1 with allowlist and models", () => {
  assert.equal(meta.key, "seedance");
  assert.equal(meta.contract, 1);
  assert.deepEqual(meta.protocols, ["video"]);
  assert.ok(meta.http.includes("ark.cn-beijing.volces.com/*"));
  assert.ok(meta.timeout_ms >= 30000);
  assert.ok(meta.models.includes("doubao-seedance-1-0-pro"));
  assert.ok(meta.description.length > 0);
});
