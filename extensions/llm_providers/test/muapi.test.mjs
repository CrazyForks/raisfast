// MuAPI 扩展契约测试 —— 断言 1:1 移植自 third/MoneyPrinterTurbo
// app/services/muapi.py 的协议行为（迁移防漂移，provider-plugins.md §9）。
//
// 运行：just test-llm-providers

import { test } from "node:test";
import assert from "node:assert/strict";
import { meta, buildSubmitRequest, parseSubmitResponse, buildQueryRequest, parseTaskResult } from "../muapi.js";

const CTX = (overrides = {}, paramOverride = {}) => ({
  provider: "muapi",
  baseUrl: "https://api.test/api/v1",
  apiKey: "mu-key",
  model: "seedance-lite-t2v",
  paramOverride,
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

test("submit posts x-api-key with endpoint route and defaults", () => {
  const spec = buildSubmitRequest(CTX());
  assert.equal(spec.method, "POST");
  assert.equal(spec.url, "https://api.test/api/v1/seedance-lite-t2v");
  assert.equal(spec.headers["x-api-key"], "mu-key");
  assert.equal(spec.body.prompt, "一只猫在草地上奔跑");
  assert.equal(spec.body.aspect_ratio, "9:16", "1080x1920 → 9:16");
  assert.equal(spec.body.resolution, "480p");
  assert.equal(spec.body.duration, 5);
});

test("duration clamped to 3..12", () => {
  const high = buildSubmitRequest(CTX({ request: { prompt: "x", seconds: "30" } }));
  assert.equal(high.body.duration, 12);
  const low = buildSubmitRequest(CTX({ request: { prompt: "x", seconds: "1" } }));
  assert.equal(low.body.duration, 3);
  const missing = buildSubmitRequest(CTX({ request: { prompt: "x", seconds: null } }));
  assert.equal(missing.body.duration, 3, "缺失/非法回落 MIN");
});

test("endpoint and aspect_ratio switch via paramOverride", () => {
  const spec = buildSubmitRequest(
    CTX({}, { endpoint: "kling-v1-master", aspect_ratio: "16:9", resolution: "720p" }),
  );
  assert.equal(spec.url, "https://api.test/api/v1/kling-v1-master");
  assert.equal(spec.body.aspect_ratio, "16:9");
  assert.equal(spec.body.resolution, "720p");
});

test("empty prompt rejected", () => {
  assert.throws(() => buildSubmitRequest(CTX({ request: { prompt: "  " } })), /prompt must not be empty/);
});

// ── 提交响应解析 ─────────────────────────────────────────────────

test("submit prefers request_id, falls back to id", () => {
  const a = parseSubmitResponse(CTX(), { status: 200, body: { request_id: "r-1" } });
  assert.equal(a.taskId, "r-1");
  const b = parseSubmitResponse(CTX(), { status: 200, body: { id: "r-2" } });
  assert.equal(b.taskId, "r-2");
});

test("submit without any id is an error (paid task state unknown)", () => {
  assert.throws(
    () => parseSubmitResponse(CTX(), { status: 200, body: {} }),
    /without returning a request id/,
  );
});

// ── 轮询 ─────────────────────────────────────────────────────────

test("buildQueryRequest polls predictions result endpoint", () => {
  const spec = buildQueryRequest(CTX({ taskId: "mu-9" }));
  assert.equal(spec.method, "GET");
  assert.equal(spec.url, "https://api.test/api/v1/predictions/mu-9/result");
  assert.equal(spec.headers["x-api-key"], "mu-key");
});

test("parseTaskResult maps completed with string and array outputs", () => {
  const single = parseTaskResult(CTX(), {
    status: 200,
    body: { status: "completed", outputs: "https://cdn.muapi.ai/a.mp4" },
  });
  assert.equal(single.status, "completed");
  assert.equal(single.url, "https://cdn.muapi.ai/a.mp4");

  const multi = parseTaskResult(CTX(), {
    status: 200,
    body: { status: "completed", outputs: ["https://cdn.muapi.ai/b.mp4", "x"] },
  });
  assert.equal(multi.url, "https://cdn.muapi.ai/b.mp4");
});

test("parseTaskResult maps failures with error detail", () => {
  const out = parseTaskResult(CTX(), {
    status: 200,
    body: { status: "failed", error: { message: "policy violation" } },
  });
  assert.equal(out.status, "failed");
  assert.equal(out.error, "policy violation");
});

test("parseTaskResult active and unknown states keep polling", () => {
  for (const state of ["queued", "pending", "processing", "weird-state"]) {
    assert.equal(
      parseTaskResult(CTX(), { status: 200, body: { status: state } }).status,
      "in_progress",
      state,
    );
  }
});

// ── meta ─────────────────────────────────────────────────────────

test("meta declares video contract v1 with allowlist", () => {
  assert.equal(meta.key, "muapi");
  assert.equal(meta.contract, 1);
  assert.deepEqual(meta.protocols, ["video"]);
  assert.ok(meta.http.includes("api.muapi.ai/*"));
  assert.ok(meta.timeout_ms >= 30000);
  assert.ok(meta.description.length > 0);
});
