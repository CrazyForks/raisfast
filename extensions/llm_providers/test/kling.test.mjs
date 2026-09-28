// Kling 扩展契约测试 —— 断言 1:1 移植自原生 crates/core/src/llm/providers/
// kling.rs 的 wiremock 测试（迁移行为防漂移，provider-plugins.md §9）。
//
// utils（jwtSignHS256/unixNow）为宿主注入的全局；node 测试用
// node:crypto 实现等价签名 + 固定时钟 stub（动态 import 在 stub 之后）。
//
// 运行：just test-llm-providers

import { test } from "node:test";
import assert from "node:assert/strict";
import crypto from "node:crypto";

// ── utils stub（真实 HMAC 签名 + 固定时钟，先于动态 import 设置）─────
const NOW = 1_700_000_000;
globalThis.utils = {
  unixNow: () => NOW,
  jwtSignHS256: (claims, secret) => {
    const b64 = (o) => Buffer.from(JSON.stringify(o)).toString("base64url");
    const input = b64({ alg: "HS256", typ: "JWT" }) + "." + b64(claims);
    return input + "." + crypto.createHmac("sha256", secret).update(input).digest("base64url");
  },
};

const { meta, buildSubmitRequest, parseSubmitResponse, buildQueryRequest, parseTaskResult } =
  await import("../kling.js");

const CTX = (overrides = {}) => ({
  provider: "kling",
  baseUrl: "https://api.test",
  apiKey: "ak-test:sk-test",
  model: "kling-v1-6",
  paramOverride: {},
  request: {
    prompt: "夜景城市",
    seconds: "5",
    size: null,
    inputReferences: [],
    callbackUrl: null,
  },
  taskId: null,
  ...overrides,
});

const ref = (name) => ({ url: `https://cdn.example.com/${name}.png` });

// ── 提交 ─────────────────────────────────────────────────────────

test("t2v posts JWT bearer with iss/exp/nbf claims", () => {
  const spec = buildSubmitRequest(CTX());
  assert.equal(spec.url, "https://api.test/v1/videos/text2video");
  assert.equal(spec.method, "POST");
  assert.equal(spec.body.model_name, "kling-v1-6");
  assert.equal(spec.body.prompt, "夜景城市");
  assert.equal(spec.body.duration, "5", "duration 直传字符串");

  const token = spec.headers.Authorization.replace("Bearer ", "");
  const parts = token.split(".");
  assert.equal(parts.length, 3, "JWT 三段");
  const claims = JSON.parse(Buffer.from(parts[1], "base64url").toString());
  assert.equal(claims.iss, "ak-test");
  assert.equal(claims.exp, NOW + 1800);
  assert.equal(claims.nbf, NOW - 5);
  // 签名可被 secret 验证（HMAC 重算一致）
  const expectSig = crypto
    .createHmac("sha256", "sk-test")
    .update(parts[0] + "." + parts[1])
    .digest("base64url");
  assert.equal(parts[2], expectSig);
});

test("i2v carries first reference and routes image2video", () => {
  const spec = buildSubmitRequest(
    CTX({ request: { prompt: "同款镜头", inputReferences: [ref("a")] } }),
  );
  assert.equal(spec.url, "https://api.test/v1/videos/image2video");
  assert.equal(spec.body.image, "https://cdn.example.com/a.png");
});

test("aspect_ratio nearest from size; absent when unparsable", () => {
  const spec = buildSubmitRequest(CTX({ request: { prompt: "x", size: "1920x1080" } }));
  assert.equal(spec.body.aspect_ratio, "16:9");

  const portrait = buildSubmitRequest(CTX({ request: { prompt: "x", size: "1080x1920" } }));
  assert.equal(portrait.body.aspect_ratio, "9:16");

  const square = buildSubmitRequest(CTX({ request: { prompt: "x", size: "960x960" } }));
  assert.equal(square.body.aspect_ratio, "1:1");

  const none = buildSubmitRequest(CTX({ request: { prompt: "x" } }));
  assert.equal(none.body.aspect_ratio, undefined);
});

test("callback_url maps to callback_url", () => {
  const spec = buildSubmitRequest(CTX({ request: { prompt: "x", callbackUrl: "https://hook.test/cb" } }));
  assert.equal(spec.body.callback_url, "https://hook.test/cb");
});

test("param_override merges into body top-level last", () => {
  const spec = buildSubmitRequest(CTX({ paramOverride: { mode: "pro", cfg_scale: 0.7 } }));
  assert.equal(spec.body.mode, "pro");
  assert.equal(spec.body.cfg_scale, 0.7);
  assert.equal(spec.body.model_name, "kling-v1-6", "基础字段仍在");
});

test("bad key format throws with clear message", () => {
  assert.throws(() => buildSubmitRequest(CTX({ apiKey: "no-separator" })), /access_key:secret_key/);
});

// ── 提交响应解析 ─────────────────────────────────────────────────

test("submit parses data.task_id and submitted→queued", () => {
  const out = parseSubmitResponse(CTX(), {
    status: 200,
    body: { code: 0, message: "Success", data: { task_id: "t-1", task_status: "submitted" } },
  });
  assert.equal(out.taskId, "t-1");
  assert.equal(out.status, "queued");
});

test("envelope code!=0 over http 200 is an error carrying code", () => {
  assert.throws(
    () =>
      parseSubmitResponse(CTX(), {
        status: 200,
        body: { code: 1000, message: "internal error" },
      }),
    /kling code 1000/,
  );
});

// ── 轮询（双路径探测）────────────────────────────────────────────

test("buildQueryRequest selects endpoint from taskData.action", () => {
  const i2v = buildQueryRequest(CTX({ taskId: "t-9", taskData: { action: "image2video" } }));
  assert.equal(i2v.url, "https://api.test/v1/videos/image2video/t-9");
  const t2v = buildQueryRequest(CTX({ taskId: "t-9", taskData: { action: "text2video" } }));
  assert.equal(t2v.url, "https://api.test/v1/videos/text2video/t-9");
});

test("parseSubmitResponse returns taskData with submit endpoint action", () => {
  const refs = { request: { prompt: "x", inputReferences: [{ url: "https://cdn.example.com/a.png" }] } };
  const out = parseSubmitResponse(CTX(refs), {
    status: 200,
    body: { code: 0, data: { task_id: "t-1", task_status: "submitted" } },
  });
  assert.equal(out.taskData.action, "image2video");
  const t2v = parseSubmitResponse(CTX(), {
    status: 200,
    body: { code: 0, data: { task_id: "t-2", task_status: "submitted" } },
  });
  assert.equal(t2v.taskData.action, "text2video");
});

test("parseTaskResult maps succeed/failed/processing and extracts video url", () => {
  const ok = parseTaskResult(CTX(), {
    status: 200,
    body: {
      code: 0,
      data: { task_status: "succeed", task_result: { videos: [{ url: "https://cdn.kling.ai/out.mp4" }] } },
    },
  });
  assert.equal(ok.status, "completed");
  assert.equal(ok.url, "https://cdn.kling.ai/out.mp4");

  const failed = parseTaskResult(CTX(), {
    status: 200,
    body: { code: 0, data: { task_status: "failed", task_status_msg: "content risk" } },
  });
  assert.equal(failed.status, "failed");
  assert.equal(failed.error, "content risk");

  const running = parseTaskResult(CTX(), {
    status: 200,
    body: { code: 0, data: { task_status: "processing" } },
  });
  assert.equal(running.status, "in_progress");
});

// ── meta ─────────────────────────────────────────────────────────

test("meta declares video contract v1 with allowlist", () => {
  assert.equal(meta.key, "kling");
  assert.equal(meta.contract, 1);
  assert.deepEqual(meta.protocols, ["video"]);
  assert.ok(meta.http.includes("api.klingai.com/*"));
  assert.ok(meta.timeout_ms >= 30000);
});
