// Vidu 扩展契约测试 —— 断言 1:1 移植自原生 crates/core/src/llm/providers/
// vidu.rs 的 wiremock 测试（迁移行为防漂移，provider-plugins.md §9）。
//
// 运行：just test-llm-providers

import { test } from "node:test";
import assert from "node:assert/strict";
import {
  meta,
  buildSubmitRequest,
  parseSubmitResponse,
  buildQueryRequest,
  parseTaskResult,
} from "../vidu.js";

const CTX = (overrides = {}) => ({
  provider: "vidu",
  baseUrl: "https://api.test",
  apiKey: "vu-key",
  model: "viduq1",
  paramOverride: {},
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

// ── 提交 ─────────────────────────────────────────────────────────

test("t2v posts Token auth with viduq1 defaults", () => {
  const spec = buildSubmitRequest(CTX());
  assert.equal(spec.url, "https://api.test/ent/v2/text2video");
  assert.equal(spec.headers.Authorization, "Token vu-key");
  assert.equal(spec.body.model, "viduq1");
  assert.equal(spec.body.prompt, "夜景城市");
  assert.equal(spec.body.duration, 5, "viduq1 default duration");
  assert.equal(spec.body.resolution, "1080p", "viduq1 locked resolution");
  assert.equal(spec.body.movement_amplitude, "auto");
  assert.equal(spec.body.images, undefined);
});

test("i2v routes img2video with images", () => {
  const spec = buildSubmitRequest(CTX({ request: { prompt: "同款镜头", inputReferences: [ref("a")] } }));
  assert.equal(spec.url, "https://api.test/ent/v2/img2video");
  assert.deepEqual(spec.body.images, ["https://cdn.example.com/a.png"]);
});

test("two refs route start-end and size maps resolution", () => {
  const spec = buildSubmitRequest(
    CTX({
      request: {
        prompt: "首尾帧",
        seconds: "5",
        size: "1920x1080",
        inputReferences: [ref("first"), ref("last")],
      },
    }),
  );
  assert.equal(spec.url, "https://api.test/ent/v2/start-end2video");
  assert.equal(spec.body.images.length, 2);
  assert.equal(spec.body.resolution, "1080p", "1920x1080 → 1080p");
  assert.equal(spec.body.duration, 5);
});

test("many refs reject non-q2 model, force viduq2 on q2", () => {
  const refs = [ref("1"), ref("2"), ref("3")];
  assert.throws(
    () => buildSubmitRequest(CTX({ model: "vidu1.5", request: { prompt: "参考", inputReferences: refs } })),
    /viduq2/,
  );
  const spec = buildSubmitRequest(CTX({ model: "viduq2", request: { prompt: "参考", inputReferences: refs } }));
  assert.equal(spec.url, "https://api.test/ent/v2/reference2video");
  assert.equal(spec.body.model, "viduq2");
});

test("vidu2.0 combo validation (no t2v; 8s only 720p)", () => {
  assert.throws(() => buildSubmitRequest(CTX({ model: "vidu2.0" })), /text-to-video/);
  assert.throws(
    () =>
      buildSubmitRequest(
        CTX({
          model: "vidu2.0",
          request: { prompt: "x", seconds: "8", size: "1920x1080", inputReferences: [ref("a")] },
        }),
      ),
    /duration 8 only allows resolution 720p/,
  );
});

test("q1 hard-locks duration 5 and resolution 1080p", () => {
  assert.throws(
    () => buildSubmitRequest(CTX({ request: { prompt: "x", seconds: "8" } })),
    /viduq1 duration must be 5/,
  );
});

test("q2 accepts 1..10 and rejects outside", () => {
  const ok = buildSubmitRequest(CTX({ model: "viduq2", request: { prompt: "x", seconds: "10" } }));
  assert.equal(ok.body.duration, 10);
  assert.throws(
    () => buildSubmitRequest(CTX({ model: "viduq2", request: { prompt: "x", seconds: "11" } })),
    /between 1 and 10/,
  );
});

// ── 提交响应解析 ─────────────────────────────────────────────────

test("submit state=failed is an error carrying err_code", () => {
  assert.throws(
    () => parseSubmitResponse(CTX(), { status: 200, body: { task_id: "vu-x", state: "failed", err_code: "CONTENT_RISK" } }),
    /CONTENT_RISK/,
  );
});

test("submit missing task_id is an error", () => {
  assert.throws(() => parseSubmitResponse(CTX(), { status: 200, body: { state: "queueing" } }), /task_id/);
});

test("submit state maps queueing→queued, processing→in_progress", () => {
  const q = parseSubmitResponse(CTX(), { status: 200, body: { task_id: "t", state: "queueing" } });
  assert.equal(q.status, "queued");
  const p = parseSubmitResponse(CTX(), { status: 200, body: { task_id: "t", state: "processing" } });
  assert.equal(p.status, "in_progress");
});

// ── 轮询 ─────────────────────────────────────────────────────────

test("buildQueryRequest polls creations endpoint", () => {
  const spec = buildQueryRequest(CTX({ taskId: "vu-9" }));
  assert.equal(spec.method, "GET");
  assert.equal(spec.url, "https://api.test/ent/v2/tasks/vu-9/creations");
  assert.equal(spec.headers.Authorization, "Token vu-key");
});

test("parseTaskResult maps success/failed/unknown", () => {
  const ok = parseTaskResult(CTX(), {
    status: 200,
    body: { state: "success", creations: [{ id: "c1", url: "https://cdn.vidu.cn/out.mp4" }] },
  });
  assert.equal(ok.status, "completed");
  assert.equal(ok.url, "https://cdn.vidu.cn/out.mp4");

  const failed = parseTaskResult(CTX(), { status: 200, body: { state: "failed", err_code: "QUOTA_EXCEEDED" } });
  assert.equal(failed.status, "failed");
  assert.equal(failed.error, "QUOTA_EXCEEDED");

  const unknown = parseTaskResult(CTX(), { status: 200, body: { state: "brand-new-state" } });
  assert.equal(unknown.status, "in_progress", "未知状态 → in_progress（sweep 容忍）");
});

// ── 分辨率归一（照抄 Rust resolution_normalization 测试）─────────

test("resolution normalization matrix", () => {
  const via = (size, model, refs = []) =>
    buildSubmitRequest(CTX({ model, request: { prompt: "x", size, inputReferences: refs } })).body.resolution;
  assert.equal(via("720P", "viduq1"), "1080p", "viduq1 locks 1080p");
  assert.equal(via("1080p", "viduq2"), "1080p");
  assert.equal(via("1280x720", "viduq2"), "720p");
  assert.equal(via("960*960", "viduq2"), "540p");
  assert.equal(via("junk", "viduq2"), "720p", "q2 default");
  assert.equal(via("", "vidu2.0", [ref("a")]), "360p", "2.0 default");
});

// ── meta ─────────────────────────────────────────────────────────

test("meta declares video contract v1 with allowlist", () => {
  assert.equal(meta.key, "vidu");
  assert.equal(meta.contract, 1);
  assert.deepEqual(meta.protocols, ["video"]);
  assert.ok(meta.http.includes("api.vidu.cn/*"));
  assert.ok(meta.timeout_ms >= 30000);
  assert.deepEqual(meta.models, ["viduq2", "viduq1", "vidu2.0", "vidu1.5"]);
  assert.ok(meta.description.length > 0);
});
