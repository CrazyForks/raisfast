// WaveSpeed 扩展契约测试 —— 断言 1:1 移植自 third/MoneyPrinterTurbo
// app/services/material.py `generate_videos_wavespeed` /
// `_wait_for_wavespeed_prediction` 的协议行为（迁移防漂移）。
//
// 运行：just test-llm-providers

import { test } from "node:test";
import assert from "node:assert/strict";
import { meta, buildSubmitRequest, parseSubmitResponse, buildQueryRequest, parseTaskResult } from "../wavespeed.js";

const BASE = "https://api.wavespeed.ai/api/v3";
const MODEL = "bytedance/seedance-2.0-fast/text-to-video";

const CTX = (overrides = {}) => ({
  provider: "wavespeed",
  baseUrl: BASE,
  apiKey: "ws-key",
  model: MODEL,
  paramOverride: {},
  request: {
    prompt: "一只猫在草地上奔跑",
    seconds: "5",
    size: "1080x1920",
    inputReferences: [],
    callbackUrl: null,
  },
  taskId: null,
  ...overrides,
});

// ── 提交构建 ─────────────────────────────────────────────────────

test("submit posts to model-routed url with bearer and clamped duration", () => {
  const spec = buildSubmitRequest(CTX());
  assert.equal(spec.method, "POST");
  assert.equal(spec.url, BASE + "/" + MODEL, "模型路由在 URL 路径");
  assert.equal(spec.headers.Authorization, "Bearer ws-key");
  assert.equal(spec.body.prompt, "一只猫在草地上奔跑");
  assert.equal(spec.body.aspect_ratio, "9:16", "1080x1920 → 9:16");
  assert.equal(spec.body.duration, 5);
});

test("duration clamped to 4..15", () => {
  const high = buildSubmitRequest(CTX({ request: { prompt: "x", seconds: "30" } }));
  assert.equal(high.body.duration, 15, "clamp to MAX");
  const low = buildSubmitRequest(CTX({ request: { prompt: "x", seconds: "2" } }));
  assert.equal(low.body.duration, 4, "clamp to MIN");
  const missing = buildSubmitRequest(CTX({ request: { prompt: "x" } }));
  assert.equal(missing.body.duration, 4, "缺失回落 MIN");
});

test("duration bounds configurable via paramOverride", () => {
  const spec = buildSubmitRequest(
    CTX({ paramOverride: { minDuration: 2, maxDuration: 15 } }, { minDuration: 2, maxDuration: 15 }),
  );
  assert.equal(spec.body.duration, 5);
});

test("aspect_ratio override wins over size derivation", () => {
  const spec = buildSubmitRequest(CTX({ paramOverride: { aspect_ratio: "16:9" } }));
  assert.equal(spec.body.aspect_ratio, "16:9");
});

test("empty prompt rejected", () => {
  assert.throws(() => buildSubmitRequest(CTX({ request: { prompt: "  " } })), /prompt must not be empty/);
});

// ── 提交响应解析 ─────────────────────────────────────────────────

test("submit parses envelope code=200 and prediction id", () => {
  const out = parseSubmitResponse(CTX(), {
    status: 200,
    body: { code: 200, message: "success", data: { id: "pred-1" } },
  });
  assert.equal(out.taskId, "pred-1");
});

test("submit rejected (business code) is an error", () => {
  assert.throws(
    () =>
      parseSubmitResponse(CTX(), {
        status: 200,
        body: { code: 404, message: "model not found", data: {} },
      }),
    /code=404/,
  );
});

test("submit missing prediction id is an error", () => {
  assert.throws(
    () => parseSubmitResponse(CTX(), { status: 200, body: { code: 200, data: {} } }),
    /prediction id/,
  );
});

// ── 轮询 ─────────────────────────────────────────────────────────

test("buildQueryRequest polls predictions result endpoint", () => {
  const spec = buildQueryRequest(CTX({ taskId: "pred-9" }));
  assert.equal(spec.method, "GET");
  assert.equal(spec.url, BASE + "/predictions/pred-9/result");
  assert.equal(spec.headers.Authorization, "Bearer ws-key");
});

test("parseTaskResult maps completed with outputs array", () => {
  const out = parseTaskResult(CTX(), {
    status: 200,
    body: { code: 200, data: { status: "completed", outputs: ["https://cdn.ws.ai/out.mp4"] } },
  });
  assert.equal(out.status, "completed");
  assert.equal(out.url, "https://cdn.ws.ai/out.mp4");
});

test("parseTaskResult maps failure states", () => {
  for (const state of ["failed", "cancelled", "timeout"]) {
    const out = parseTaskResult(CTX(), {
      status: 200,
      body: { code: 200, data: { status: state } },
    });
    assert.equal(out.status, "failed", state);
  }
});

test("parseTaskResult maps running/queued to in_progress", () => {
  for (const state of ["running", "queued", "preparing"]) {
    const out = parseTaskResult(CTX(), {
      status: 200,
      body: { code: 200, data: { status: state } },
    });
    assert.equal(out.status, "in_progress", state);
  }
});

test("parseTaskResult envelope code!=200 is status-unknown error", () => {
  assert.throws(
    () =>
      parseTaskResult(CTX(), {
        status: 200,
        body: { code: 404, message: "prediction not found", data: null },
      }),
    /status unknown/,
  );
});

// ── meta ─────────────────────────────────────────────────────────

test("meta declares video contract v1 with allowlist and models", () => {
  assert.equal(meta.key, "wavespeed");
  assert.equal(meta.contract, 1);
  assert.deepEqual(meta.protocols, ["video"]);
  assert.ok(meta.http.includes("api.wavespeed.ai/*"));
  assert.ok(meta.timeout_ms >= 30000);
  assert.ok(meta.models.includes(MODEL));
  assert.ok(meta.description.length > 0);
});
