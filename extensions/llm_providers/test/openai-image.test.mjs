// OpenAI 兼容文生图扩展契约测试 —— 断言 1:1 移植自
// third/MoneyPrinterTurbo app/services/material.py 的协议行为
//（迁移防漂移，provider-plugins.md §9）。
//
// 运行：just test-llm-providers

import { test } from "node:test";
import assert from "node:assert/strict";
import { meta, buildImageRequest, parseImageResponse } from "../openai-image.js";

const CTX = (overrides = {}, paramOverride = {}) => ({
  provider: "openai-image",
  baseUrl: "https://api.test/v1",
  apiKey: "sk-key",
  model: "gpt-image-1",
  paramOverride,
  request: {
    prompt: "赛博朋克城市夜景",
    n: 1,
    size: "1024x1536",
    inputReferences: [],
    callbackUrl: null,
  },
  taskId: null,
  ...overrides,
});

// ── 生成构建 ─────────────────────────────────────────────────────

test("build posts model/prompt/n/size with bearer", () => {
  const spec = buildImageRequest(CTX());
  assert.equal(spec.method, "POST");
  assert.equal(spec.url, "https://api.test/v1/images/generations");
  assert.equal(spec.headers.Authorization, "Bearer sk-key");
  assert.equal(spec.body.model, "gpt-image-1");
  assert.equal(spec.body.prompt, "赛博朋克城市夜景");
  assert.equal(spec.body.n, 1);
  assert.equal(spec.body.size, "1024x1536");
});

test("size omitted when absent (provider default)", () => {
  const spec = buildImageRequest(CTX({ request: { prompt: "x", n: 2, size: null } }));
  assert.equal(spec.body.n, 2);
  assert.equal(spec.body.size, undefined);
});

test("empty apiKey omits Authorization header (local gateways)", () => {
  const spec = buildImageRequest(CTX({ apiKey: null }));
  assert.equal(spec.headers.Authorization, undefined);
});

test("empty prompt rejected", () => {
  assert.throws(() => buildImageRequest(CTX({ request: { prompt: "  " } })), /prompt must not be empty/);
});

// ── 响应解析 ─────────────────────────────────────────────────────

test("parse maps b64_json entries", () => {
  const out = parseImageResponse(CTX(), {
    status: 200,
    body: { data: [{ b64_json: "QUJD" }, { b64_json: "REVG" }] },
  });
  assert.deepEqual(out.images, [{ b64Json: "QUJD" }, { b64Json: "REVG" }]);
});

test("parse maps url entries (http(s) only)", () => {
  const out = parseImageResponse(CTX(), {
    status: 200,
    body: { data: [{ url: "https://cdn.test/a.png" }] },
  });
  assert.deepEqual(out.images, [{ url: "https://cdn.test/a.png" }]);
});

test("parse skips entries with neither url nor b64_json", () => {
  const out = parseImageResponse(CTX(), {
    status: 200,
    body: { data: [{ junk: true }, { url: "https://cdn.test/b.png" }] },
  });
  assert.deepEqual(out.images, [{ url: "https://cdn.test/b.png" }]);
});

test("parse throws on missing/empty data array", () => {
  assert.throws(() => parseImageResponse(CTX(), { status: 200, body: {} }), /no data array/);
  assert.throws(() => parseImageResponse(CTX(), { status: 200, body: { data: [] } }), /no data array/);
});

test("parse throws when no entry has url or b64_json", () => {
  assert.throws(
    () => parseImageResponse(CTX(), { status: 200, body: { data: [{ junk: 1 }] } }),
    /neither url nor b64_json/,
  );
});

// ── meta ─────────────────────────────────────────────────────────

test("meta declares image protocol with generous timeout", () => {
  assert.equal(meta.key, "openai-image");
  assert.equal(meta.contract, 1);
  assert.deepEqual(meta.protocols, ["image"]);
  assert.ok(meta.http.includes("api.openai.com/*"));
  assert.ok(meta.timeout_ms >= 300000, "同步生成可能数十秒");
  assert.ok(meta.description.length > 0);
});
