// CogVideo 扩展契约测试：喂录制的 request/response JSON 对，断言 build/parse
// 输出（provider-plugins.md §9 契约防回归）。纯函数、零网络、零内核。
//
// 运行：just test-llm-providers（node --test，秒级）

import { test } from "node:test";
import assert from "node:assert/strict";
import { meta, buildSubmitRequest, parseSubmitResponse, buildQueryRequest, parseTaskResult } from "../cogvideo.js";

const CTX = (overrides = {}) => ({
  provider: "cogvideo",
  baseUrl: "https://api.test",
  apiKey: "zk-test-key",
  model: "cogvideox-2",
  paramOverride: {},
  request: {
    prompt: "一只熊猫吃竹子",
    seconds: "5",
    size: null,
    inputReferences: [],
    callbackUrl: null,
  },
  taskId: null,
  ...overrides,
});

test("meta declares video contract v1 with allowlist", () => {
  assert.equal(meta.key, "cogvideo");
  assert.equal(meta.contract, 1);
  assert.deepEqual(meta.protocols, ["video"]);
  assert.ok(meta.http.includes("open.bigmodel.cn/*"));
  assert.ok(meta.timeout_ms >= 30000);
  assert.deepEqual(meta.models, ["cogvideox-2", "cogvideox", "cogvideox-flash"]);
  assert.ok(meta.description.length > 0);
});

test("buildSubmitRequest posts Bearer auth with model/prompt/duration", () => {
  const spec = buildSubmitRequest(CTX());
  assert.equal(spec.method, "POST");
  assert.equal(spec.url, "https://api.test/api/paas/v4/videos/generations");
  assert.equal(spec.headers.Authorization, "Bearer zk-test-key");
  assert.equal(spec.body.model, "cogvideox-2");
  assert.equal(spec.body.prompt, "一只熊猫吃竹子");
  assert.equal(spec.body.duration, 5);
});

test("buildSubmitRequest omits duration for non-numeric seconds", () => {
  const spec = buildSubmitRequest(CTX({ request: { prompt: "x", seconds: null } }));
  assert.equal(spec.body.duration, undefined);
});

test("parseSubmitResponse extracts task id (id field)", () => {
  const out = parseSubmitResponse(CTX(), {
    status: 200,
    body: { id: "task-1", task_status: "PROCESSING" },
  });
  assert.deepEqual(out, { taskId: "task-1" });
});

test("parseSubmitResponse extracts task id (task_id field)", () => {
  const out = parseSubmitResponse(CTX(), {
    status: 200,
    body: { task_id: "task-2", task_status: "PROCESSING" },
  });
  assert.equal(out.taskId, "task-2");
});

test("parseSubmitResponse throws on http error and on missing id", () => {
  assert.throws(() => parseSubmitResponse(CTX(), { status: 401, body: { error: "bad key" } }));
  assert.throws(() => parseSubmitResponse(CTX(), { status: 200, body: {} }));
});

test("buildQueryRequest polls with taskId in url", () => {
  const spec = buildQueryRequest(CTX({ taskId: "task-9" }));
  assert.equal(spec.method, "GET");
  assert.equal(spec.url, "https://api.test/api/paas/v4/videos/generations/task-9");
  assert.equal(spec.headers.Authorization, "Bearer zk-test-key");
});

test("parseTaskResult maps SUCCESS to completed with url", () => {
  const out = parseTaskResult(CTX(), {
    status: 200,
    body: { task_status: "SUCCESS", video_result: [{ url: "https://cdn.test/out.mp4" }] },
  });
  assert.equal(out.status, "completed");
  assert.equal(out.url, "https://cdn.test/out.mp4");
});

test("parseTaskResult maps PROCESSING to in_progress and FAIL to failed", () => {
  assert.equal(parseTaskResult(CTX(), { status: 200, body: { task_status: "PROCESSING" } }).status, "in_progress");
  const fail = parseTaskResult(CTX(), { status: 200, body: { task_status: "FAIL" } });
  assert.equal(fail.status, "failed");
  assert.ok(fail.error.length > 0);
});

test("parseTaskResult treats unknown status as in_progress (sweep tolerance)", () => {
  assert.equal(parseTaskResult(CTX(), { status: 200, body: { task_status: "NEW_STATE" } }).status, "in_progress");
});
