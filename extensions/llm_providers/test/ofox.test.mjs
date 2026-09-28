// OFox 扩展契约测试 —— 断言 1:1 移植自 third/MoneyPrinterTurbo
// app/services/ofox.py 的协议行为（迁移防漂移，provider-plugins.md §9）。
//
// 运行：just test-llm-providers

import { test } from "node:test";
import assert from "node:assert/strict";
import { meta, buildSubmitRequest, parseSubmitResponse, buildQueryRequest, parseTaskResult } from "../ofox.js";

const CTX = (overrides = {}, paramOverride = {}) => ({
  provider: "ofox",
  baseUrl: "https://api.test/v1",
  apiKey: "ofox-key",
  model: "bytedance/seedance-2.0-fast",
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

test("submit posts bearer with provider.type and defaults", () => {
  const spec = buildSubmitRequest(CTX());
  assert.equal(spec.method, "POST");
  assert.equal(spec.url, "https://api.test/v1/videos");
  assert.equal(spec.headers.Authorization, "Bearer ofox-key");
  assert.equal(spec.body.model, "bytedance/seedance-2.0-fast");
  assert.equal(spec.body.prompt, "一只猫在草地上奔跑");
  assert.equal(spec.body.resolution, "720p");
  assert.equal(spec.body.aspect_ratio, "9:16", "1080x1920 → 9:16");
  assert.equal(spec.body.provider.type, "byteplus", "默认钉定 byteplus 通道");
});

test("duration clamped to 4..15 (seedance-2.0-fast 服务端实测)", () => {
  const high = buildSubmitRequest(CTX({ request: { prompt: "x", seconds: "30" } }));
  assert.equal(high.body.duration, 15);
  const low = buildSubmitRequest(CTX({ request: { prompt: "x", seconds: "2" } }));
  assert.equal(low.body.duration, 4);
  const missing = buildSubmitRequest(CTX({ request: { prompt: "x" } }));
  assert.equal(missing.body.duration, 4);
});

test("model/providerType/resolution switch via paramOverride", () => {
  const spec = buildSubmitRequest(
    CTX(
      { request: { prompt: "x", seconds: "10", size: "1920x1080" } },
      { model: "alibaba/wan-2.7", providerType: "alibaba", resolution: "1080p" },
    ),
  );
  assert.equal(spec.body.model, "alibaba/wan-2.7");
  assert.equal(spec.body.provider.type, "alibaba");
  assert.equal(spec.body.resolution, "1080p");
  assert.equal(spec.body.aspect_ratio, "16:9");
});

test("empty prompt rejected", () => {
  assert.throws(() => buildSubmitRequest(CTX({ request: { prompt: "  " } })), /prompt must not be empty/);
});

// ── 提交响应解析 ─────────────────────────────────────────────────

test("submit parses body.id", () => {
  const out = parseSubmitResponse(CTX(), { status: 200, body: { id: "ofx-1" } });
  assert.equal(out.taskId, "ofx-1");
});

test("submit without id is an error (paid task state unknown)", () => {
  assert.throws(
    () => parseSubmitResponse(CTX(), { status: 200, body: {} }),
    /without returning a task id/,
  );
});

// ── 轮询 ─────────────────────────────────────────────────────────

test("buildQueryRequest polls videos/{taskId} with bearer", () => {
  const spec = buildQueryRequest(CTX({ taskId: "ofx-9" }));
  assert.equal(spec.method, "GET");
  assert.equal(spec.url, "https://api.test/v1/videos/ofx-9");
  assert.equal(spec.headers.Authorization, "Bearer ofox-key");
});

test("parseTaskResult prefers mirror_urls over unsigned_urls", () => {
  const out = parseTaskResult(CTX(), {
    status: 200,
    body: {
      status: "completed",
      mirror_urls: ["https://cdn.ofox.ai/mirror.mp4"],
      unsigned_urls: ["https://upstream.example/tmp.mp4"],
    },
  });
  assert.equal(out.status, "completed");
  assert.equal(out.url, "https://cdn.ofox.ai/mirror.mp4");
});

test("parseTaskResult falls back to unsigned_urls", () => {
  const out = parseTaskResult(CTX(), {
    status: 200,
    body: { status: "completed", unsigned_urls: ["https://upstream.example/tmp.mp4"] },
  });
  assert.equal(out.url, "https://upstream.example/tmp.mp4");
});

test("completed without downloadable video maps to failed", () => {
  const out = parseTaskResult(CTX(), { status: 200, body: { status: "completed", mirror_urls: [] } });
  assert.equal(out.status, "failed");
  assert.match(out.error, /without a downloadable video/);
});

test("parseTaskResult maps failures with detail", () => {
  const out = parseTaskResult(CTX(), {
    status: 200,
    body: { status: "failed", error: { message: "policy violation" } },
  });
  assert.equal(out.status, "failed");
  assert.equal(out.error, "policy violation");
});

test("parseTaskResult active states and unknown keep polling", () => {
  for (const state of ["pending", "queued", "in_progress", "weird-state"]) {
    assert.equal(
      parseTaskResult(CTX(), { status: 200, body: { status: state } }).status,
      "in_progress",
      state,
    );
  }
});

// ── meta ─────────────────────────────────────────────────────────

test("meta declares video contract v1 with allowlist and models", () => {
  assert.equal(meta.key, "ofox");
  assert.equal(meta.contract, 1);
  assert.deepEqual(meta.protocols, ["video"]);
  assert.ok(meta.http.includes("ofox.ai/*"));
  assert.ok(meta.timeout_ms >= 30000);
  assert.ok(meta.models.includes("bytedance/seedance-2.0-fast"));
  assert.ok(meta.description.length > 0);
});
