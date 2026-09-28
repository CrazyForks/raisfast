// MiniMax 扩展契约测试 —— 断言 1:1 移植自原生 crates/core/src/llm/providers/
// minimax.rs 的 wiremock 测试（迁移行为防漂移，provider-plugins.md §9）。
//
// utils 未被 minimax 使用（无 JWT/签名需求），无需 stub。
//
// 运行：just test-llm-providers

import { test } from "node:test";
import assert from "node:assert/strict";
import {
  meta,
  buildChatRequest,
  parseChatResponse,
  buildSpeechRequest,
  parseSpeechResponse,
  buildMusicRequest,
  parseMusicResponse,
  buildSubmitRequest,
  parseSubmitResponse,
  buildQueryRequest,
  parseTaskResult,
} from "../minimax.js";

const CTX = (overrides = {}, paramOverride = {}) => ({
  provider: "minimax",
  baseUrl: "https://api.test",
  apiKey: "mm-key",
  model: "MiniMax-H3",
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

// ── chat（chatcompletion_v2）────────────────────────────────────────

test("chat posts to chatcompletion_v2 with tools and params", () => {
  const spec = buildChatRequest(
    CTX(
      {
        request: {
          messages: [
            { role: "system", content: "你是助手" },
            { role: "user", content: "杭州天气", images: ["https://img.test/w.png"] },
          ],
          temperature: 0.5,
          maxTokens: 512,
          tools: [{ name: "weather", description: "查天气", parameters: { type: "object" }, category: "x" }],
        },
      },
      { temperature: 0.9 },
    ),
  );
  assert.equal(spec.method, "POST");
  assert.equal(spec.url, "https://api.test/v1/text/chatcompletion_v2");
  assert.equal(spec.headers.Authorization, "Bearer mm-key");
  assert.equal(spec.body.model, "MiniMax-H3");
  assert.equal(spec.body.messages[0].content, "你是助手");
  assert.equal(spec.body.messages[1].content[0].type, "text");
  assert.equal(spec.body.messages[1].content[1].image_url.url, "https://img.test/w.png");
  assert.equal(spec.body.temperature, 0.9, "paramOverride 覆盖生效");
  assert.equal(spec.body.max_tokens, 512);
  assert.equal(spec.body.tools[0].type, "function");
  assert.equal(spec.body.tools[0].function.name, "weather");
});

test("parseChatResponse maps choices/usage/tool_calls", () => {
  const out = parseChatResponse(CTX(), {
    status: 200,
    body: {
      choices: [
        {
          message: {
            content: "晴",
            tool_calls: [
              { id: "c1", function: { name: "weather", arguments: "{\"city\":\"hangzhou\"}" } },
            ],
          },
        },
      ],
      usage: { prompt_tokens: 20, completion_tokens: 4 },
    },
  });
  assert.equal(out.text, "晴");
  assert.equal(out.toolCalls[0].name, "weather");
  assert.equal(out.usage.inputTokens, 20);
  assert.equal(out.usage.outputTokens, 4);
});

test("chat base_resp reject throws", () => {
  assert.throws(
    () =>
      parseChatResponse(CTX(), {
        status: 200,
        body: { base_resp: { status_code: 1004, status_msg: "auth failed" } },
      }),
    /minimax code 1004/,
  );
});

// ── speech / music（GroupId 强制；hex 音频）────────────────────────

test("speech requires GroupId in key", () => {
  assert.throws(
    () => buildSpeechRequest(CTX({ apiKey: "mm-key" }, {}), ),
    /GroupId is mandatory/,
  );
});

test("speech posts t2a_v2 with GroupId and audio_setting", () => {
  const spec = buildSpeechRequest(
    CTX({ apiKey: "mm-key:group-1", request: { text: "你好世界", voice: "female" } }),
  );
  assert.equal(spec.url, "https://api.test/v1/t2a_v2?GroupId=group-1");
  assert.equal(spec.headers.Authorization, "Bearer mm-key");
  assert.equal(spec.body.text, "你好世界");
  assert.equal(spec.body.voice_setting.voice_id, "female");
  assert.equal(spec.body.audio_setting.format, "mp3");
});

test("parseSpeechResponse extracts hex audio", () => {
  const out = parseSpeechResponse(CTX(), {
    status: 200,
    body: { data: { audio: "494433" }, base_resp: { status_code: 0 } },
  });
  assert.equal(out.audioHex, "494433");
});

test("music requires GroupId and posts prompt/lyrics", () => {
  assert.throws(
    () => buildMusicRequest(CTX({ apiKey: "mm-key" })),
    /GroupId is mandatory/,
  );
  const spec = buildMusicRequest(
    CTX({ apiKey: "k:g1" }, {}),
  );
  assert.equal(spec.url, "https://api.test/v1/music_generation?GroupId=g1");
  assert.equal(spec.body.prompt, "夜景城市");
  assert.equal(spec.body.lyrics, undefined);
});

// ── video 提交（V2 H3 / V1 flat）──────────────────────────────────

test("H3 submit posts V2 content array with validated params", () => {
  const spec = buildSubmitRequest(
    CTX(
      {
        model: "MiniMax-H3",
        request: { prompt: "夜景城市", seconds: "6", size: "2k", inputReferences: [ref("a")] },
      },
      { callback_url: "https://hook.test/cb" },
    ),
  );
  assert.equal(spec.url, "https://api.test/v2/video_generation");
  assert.equal(spec.body.content[0].type, "text");
  assert.equal(spec.body.content[1].role, "first_frame");
  assert.equal(spec.body.resolution, "2K");
  assert.equal(spec.body.duration, 6);
  assert.equal(spec.body.ratio, "adaptive", "有参考图 → adaptive");
  assert.equal(spec.body.callback_url, "https://hook.test/cb");
});

test("H3 duration out of range is rejected", () => {
  assert.throws(
    () => buildSubmitRequest(CTX({ model: "MiniMax-H3", request: { prompt: "x", seconds: "20" } })),
    /duration must be an integer between 4 and 15/,
  );
});

test("H3 adaptive ratio without visual is rejected", () => {
  assert.throws(
    () =>
      buildSubmitRequest(
        CTX(
          { model: "MiniMax-H3", request: { prompt: "x" } },
          { ratio: "adaptive" },
        ),
      ),
    /adaptive requires an image or video input/,
  );
});

test("V1 submit maps first frame and default resolution", () => {
  const spec = buildSubmitRequest(
    CTX({
      model: "MiniMax-Hailuo-02",
      request: { prompt: "同款镜头", inputReferences: [ref("a")] },
    }),
  );
  assert.equal(spec.url, "https://api.test/v1/video_generation");
  assert.equal(spec.body.first_frame_image, "https://cdn.example.com/a.png");
  assert.equal(spec.body.resolution, "768P", "Hailuo-02 默认 768P");
  assert.equal(spec.body.duration, 6, "V1 默认 6s");
});

test("V1 720P on modern hailuo maps to 768P", () => {
  const spec = buildSubmitRequest(CTX({ model: "MiniMax-Hailuo-02", request: { prompt: "x", size: "720p" } }));
  assert.equal(spec.body.resolution, "768P");
});

test("empty prompt rejected", () => {
  assert.throws(
    () => buildSubmitRequest(CTX({ model: "MiniMax-H3", request: { prompt: "  " } })),
    /prompt must not be empty/,
  );
});

// ── video 解析 ───────────────────────────────────────────────────

test("H3 submit parses top-level status and task_id", () => {
  const out = parseSubmitResponse(CTX({ model: "MiniMax-H3" }), {
    status: 200,
    body: { task_id: "v-1", status: "queued" },
  });
  assert.equal(out.taskId, "v-1");
  assert.equal(out.status, "queued");
});

test("H3 query reads task.status and error chain", () => {
  const ok = parseTaskResult(CTX({ model: "MiniMax-H3", taskId: "v-1" }), {
    status: 200,
    body: { task: { status: "succeeded", content: { url: "https://cdn.test/out.mp4" } } },
  });
  assert.equal(ok.status, "completed");
  assert.equal(ok.url, "https://cdn.test/out.mp4");

  const failed = parseTaskResult(CTX({ model: "MiniMax-H3", taskId: "v-2" }), {
    status: 200,
    body: { task: { status: "failed", error: { message: "policy" } } },
  });
  assert.equal(failed.status, "failed");
  assert.equal(failed.error, "policy");
});

test("V1 query returns authed download url with headers", () => {
  const out = parseTaskResult(CTX({ model: "MiniMax-Hailuo-02", taskId: "v-3" }), {
    status: 200,
    body: { status: "Success", file_id: "f-1" },
  });
  assert.equal(out.status, "completed");
  assert.equal(out.url, "https://api.test/v1/files/download?file_id=f-1");
  assert.equal(out.downloadHeaders.Authorization, "Bearer mm-key");
});

test("V1 status vocabulary maps Processing/Success/Fail", () => {
  assert.equal(
    parseTaskResult(CTX({ model: "MiniMax-Hailuo-02", taskId: "t" }), {
      status: 200,
      body: { status: "Processing" },
    }).status,
    "in_progress",
  );
  assert.equal(
    parseTaskResult(CTX({ model: "MiniMax-Hailuo-02", taskId: "t" }), {
      status: 200,
      body: { status: "Fail", base_resp: { status_msg: "bad" } },
    }).error,
    "bad",
  );
});

// ── meta ─────────────────────────────────────────────────────────

test("meta declares four-modal contract v1", () => {
  assert.equal(meta.key, "minimax");
  assert.equal(meta.contract, 1);
  assert.deepEqual(meta.protocols, ["chat", "speech", "music", "video"]);
  assert.ok(meta.http.includes("api.minimaxi.com/*"));
  assert.ok(meta.timeout_ms >= 30000);
  assert.ok(meta.models.includes("MiniMax-H3"));
  assert.ok(meta.description.length > 0);
});
