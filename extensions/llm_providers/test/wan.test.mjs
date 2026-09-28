// Wan 扩展契约测试 —— 断言 1:1 移植自原生 crates/core/src/llm/providers/
// wan.rs 的 wiremock 测试（迁移行为防漂移，provider-plugins.md §9）。
//
// 运行：just test-llm-providers

import { test } from "node:test";
import assert from "node:assert/strict";
import { meta, buildSubmitRequest, parseSubmitResponse, buildQueryRequest, parseTaskResult } from "../wan.js";

const CTX = (overrides = {}, paramOverride = {}) => ({
  provider: "wan",
  baseUrl: "https://api.test",
  apiKey: "ds-key",
  model: "wan2.7-t2v",
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

test("t2v submit: async envelope + defaults + size tiers", () => {
  const spec = buildSubmitRequest(CTX({ request: { prompt: "夜景城市", seconds: "5", size: "1920x1080" } }));
  assert.equal(spec.method, "POST");
  assert.equal(spec.url, "https://api.test/api/v1/services/aigc/video-generation/video-synthesis");
  assert.equal(spec.headers.Authorization, "Bearer ds-key");
  assert.equal(spec.headers["X-DashScope-Async"], "enable");
  assert.equal(spec.body.model, "wan2.7-t2v");
  assert.equal(spec.body.input.prompt, "夜景城市");
  assert.equal(spec.body.parameters.resolution, "1080P");
  assert.equal(spec.body.parameters.ratio, "16:9");
  assert.equal(spec.body.parameters.duration, 5);
  assert.equal(spec.body.parameters.prompt_extend, true);
});

test("legacy size model emits pixel size (kind=size)", () => {
  const spec = buildSubmitRequest(CTX({ model: "wan2.6-t2v", request: { prompt: "x", seconds: "5", size: "1280x720" } }));
  assert.equal(spec.body.parameters.size, "1280*720");
  assert.equal(spec.body.parameters.resolution, undefined);
  assert.equal(spec.body.parameters.ratio, undefined);
});

test("i2v routes image2video with img_url", () => {
  const spec = buildSubmitRequest(CTX({ model: "wan2.6-i2v", request: { prompt: "同款镜头", inputReferences: [ref("a")] } }));
  assert.equal(spec.url, "https://api.test/api/v1/services/aigc/image2video/video-synthesis");
  assert.equal(spec.body.input.img_url, "https://cdn.example.com/a.png");
});

test("wan2.7-i2v media first/last frame", () => {
  const spec = buildSubmitRequest(
    CTX({ model: "wan2.7-i2v", request: { prompt: "首尾帧", inputReferences: [ref("first"), ref("last")] } }),
  );
  assert.equal(spec.body.input.media[0].type, "first_frame");
  assert.equal(spec.body.input.media[1].type, "last_frame");
});

test("wan3.0 reference images and smart duration", () => {
  const spec = buildSubmitRequest(
    CTX(
      { model: "wan3.0-video", request: { prompt: "参考生成", seconds: "-1", inputReferences: [ref("1"), ref("2"), ref("3")] } },
    ),
  );
  assert.equal(spec.body.parameters.duration, -1);
  assert.equal(spec.body.input.media.length, 3);
  assert.equal(spec.body.input.media[0].type, "reference_image");
});

test("wan2.1 prefix normalizes to wanx2.1 profile", () => {
  // wan2.1-i2v-turbo durations = [3,4,5]；7 → 拒绝（证明 profile 命中 wanx2.1 表）
  assert.throws(
    () => buildSubmitRequest(CTX({ model: "wan2.1-i2v-turbo", request: { prompt: "x", seconds: "7", inputReferences: [ref("a")] } })),
    /duration must be one of 3, 4, 5/,
  );
});

test("date-suffixed model resolves to base profile", () => {
  const spec = buildSubmitRequest(CTX({ model: "wan2.7-t2v-2026-04-25" }));
  assert.equal(spec.body.model, "wan2.7-t2v-2026-04-25", "上游收到原始模型名");
  assert.equal(spec.body.parameters.resolution, "1080P");
});

// ── 校验（防超预期费用）──────────────────────────────────────────

test("unsupported model is rejected", () => {
  assert.throws(() => buildSubmitRequest(CTX({ model: "wan9.9-t2v" })), /unsupported model/);
});

test("duration enum rejected off paid tier", () => {
  assert.throws(
    () => buildSubmitRequest(CTX({ model: "wan2.5-t2v-preview", request: { prompt: "x", seconds: "7" } })),
    /duration must be one of 5, 10/,
  );
});

test("smart duration rejected on non-wan30", () => {
  assert.throws(
    () => buildSubmitRequest(CTX({ model: "wan2.7-t2v", request: { prompt: "x", seconds: "-1" } })),
    /duration must be an integer between 2 and 15/,
  );
});

test("speech model unsupported (no audio surface)", () => {
  assert.throws(() => buildSubmitRequest(CTX({ model: "wan2.2-s2v" })), /s2v/);
});

test("param_override merges and is validated", () => {
  const ok = buildSubmitRequest(CTX({}, { resolution: "720P", seed: 42 }));
  assert.equal(ok.body.parameters.resolution, "720P");
  assert.equal(ok.body.parameters.seed, 42);

  assert.throws(
    () => buildSubmitRequest(CTX({}, { resolution: "4K" })),
    /resolution must be one of/,
  );
});

// ── 提交响应解析 ─────────────────────────────────────────────────

test("submit parses task id and pending state", () => {
  const out = parseSubmitResponse(CTX(), {
    status: 200,
    body: { request_id: "r-1", output: { task_id: "wan-t-1", task_status: "PENDING" } },
  });
  assert.equal(out.taskId, "wan-t-1");
  assert.equal(out.status, "queued");
});

test("submit envelope error (string code) maps to error", () => {
  assert.throws(
    () => parseSubmitResponse(CTX(), { status: 200, body: { code: "InvalidApiKey", message: "Invalid API-key" } }),
    /InvalidApiKey/,
  );
});

// ── 轮询 ─────────────────────────────────────────────────────────

test("buildQueryRequest polls task endpoint with bearer", () => {
  const spec = buildQueryRequest(CTX({ taskId: "wan-t-1" }));
  assert.equal(spec.method, "GET");
  assert.equal(spec.url, "https://api.test/api/v1/tasks/wan-t-1");
  assert.equal(spec.headers.Authorization, "Bearer ds-key");
});

test("parseTaskResult maps SUCCEEDED with video_url", () => {
  const out = parseTaskResult(CTX(), {
    status: 200,
    body: { output: { task_status: "SUCCEEDED", video_url: "https://oss.example.com/out.mp4" } },
  });
  assert.equal(out.status, "completed");
  assert.equal(out.url, "https://oss.example.com/out.mp4");
});

test("parseTaskResult maps results.video_url fallback", () => {
  const out = parseTaskResult(CTX(), {
    status: 200,
    body: { output: { task_status: "SUCCEEDED", results: { video_url: "https://oss.example.com/fb.mp4" } } },
  });
  assert.equal(out.url, "https://oss.example.com/fb.mp4");
});

test("parseTaskResult maps FAILED with reason chain", () => {
  const out = parseTaskResult(CTX(), {
    status: 200,
    body: {
      output: { task_status: "FAILED", code: "InvalidParameter", message: "bad image" },
    },
  });
  assert.equal(out.status, "failed");
  assert.match(out.error, /bad image/);
  assert.match(out.error, /InvalidParameter/);
});

test("parseTaskResult maps RUNNING/PENDING and tolerates unknown states", () => {
  assert.equal(parseTaskResult(CTX(), { status: 200, body: { output: { task_status: "RUNNING" } } }).status, "in_progress");
  assert.equal(parseTaskResult(CTX(), { status: 200, body: { output: { task_status: "PENDING" } } }).status, "queued");
  assert.equal(parseTaskResult(CTX(), { status: 200, body: { output: { task_status: "WEIRD" } } }).status, "in_progress");
});

// ── meta ─────────────────────────────────────────────────────────

test("meta declares video contract v1 with allowlist and models", () => {
  assert.equal(meta.key, "wan");
  assert.equal(meta.contract, 1);
  assert.deepEqual(meta.protocols, ["video"]);
  assert.ok(meta.http.includes("dashscope.aliyuncs.com/*"));
  assert.ok(meta.timeout_ms >= 30000);
  assert.ok(meta.models.includes("wan3.0-video"));
  assert.ok(meta.description.length > 0);
});
