// Unit tests for services/autoreply.js — bot gate, handoff triggers,
// context window and LLM call. Runs against the `sdk` mock.

import { test, beforeEach } from "node:test";
import assert from "node:assert/strict";

import {
  __reset,
  __seed,
  __rows,
  __seedReceipt,
  __setLlReply,
  __emitted,
} from "./sdk-mock.mjs";
import { onAutoreply } from "../services/autoreply.js";

function seedBot(overrides = {}) {
  __seed("chat_bots", [{
    id: "1",
    name: "Test Bot",
    enabled: true,
    mode: "full",
    autoreply: { client: "glm", op: "chat", context_window: 10 },
    handoff: { keywords: ["转人工", "人工客服"] },
    ...overrides,
  }]);
}

function seedConv(overrides = {}) {
  __seed("chat_conversations", [{
    id: "100",
    contact_id: "50",
    status: "pending",
    bot_status: "active",
    last_message_role: "user",
    ...overrides,
  }]);
}

const JOB = {
  trace_id: "trace-1",
  channel_key: "web",
  conversation_id: "100",
  bot_id: "1",
};

beforeEach(() => __reset());

test("skips when conversation bot_status is disabled (agent took over)", () => {
  seedBot();
  seedConv({ bot_status: "disabled" });
  const out = onAutoreply(JOB);
  assert.equal(out.skipped, "bot_disabled");
  assert.equal(__rows("chat_messages").length, 0);
});

test("fallback mode aborts when an agent replied last", () => {
  seedBot({ mode: "fallback" });
  seedConv({ last_message_role: "agent" });
  const out = onAutoreply(JOB);
  assert.equal(out.skipped, "agent_took_over");
  assert.equal(__rows("chat_messages").length, 0);
});

test("visitor handoff keyword transfers to a human without replying", () => {
  seedBot();
  seedConv();
  __seedReceipt("trace-1", "ch-1", { sender: "vis", payload: { body: "我要转人工" } });
  const out = onAutoreply(JOB);
  assert.equal(out.handoff, true);
  // conversation flipped to open + bot disabled
  const conv = __rows("chat_conversations")[0];
  assert.equal(conv.status, "open");
  assert.equal(conv.bot_status, "disabled");
  // no assistant message written
  assert.equal(__rows("chat_messages").length, 0);
});

test("replies via LLM and writes an assistant message", () => {
  seedBot();
  seedConv();
  __setLlReply("你好，请问有什么可以帮您？");
  __seedReceipt("trace-1", "ch-1", { sender: "vis", payload: { body: "你好" } });
  const out = onAutoreply(JOB);
  assert.equal(out.conversation_id, "100");
  const msgs = __rows("chat_messages");
  assert.equal(msgs.length, 1);
  assert.equal(msgs[0].role, "assistant");
  assert.equal(msgs[0].body, "你好，请问有什么可以帮您？");
  assert.equal(msgs[0].receipt_id, "trace-1");
  // emitted chat.message.created
  const events = __emitted().filter((e) => e.kind === "event");
  assert.ok(events.some((e) => e.type === "chat.message.created" && e.data.role === "assistant"));
});

test("first_line mode hands off after replying", () => {
  seedBot({ mode: "first_line" });
  seedConv();
  __setLlReply("好的，我来帮您。");
  __seedReceipt("trace-1", "ch-1", { sender: "vis", payload: { body: "帮我看看" } });
  onAutoreply(JOB);
  const conv = __rows("chat_conversations")[0];
  assert.equal(conv.status, "open");
  assert.equal(conv.bot_status, "disabled");
  // assistant message still written
  assert.equal(__rows("chat_messages").length, 1);
});

test("context window caps history to bot config", () => {
  seedBot({ autoreply: { client: "glm", op: "chat", context_window: 3 } });
  seedConv();
  __seed("chat_messages", [
    { id: "1", conversation_id: "100", role: "user", body: "m1" },
    { id: "2", conversation_id: "100", role: "agent", body: "m2" },
    { id: "3", conversation_id: "100", role: "assistant", body: "m3" },
    { id: "4", conversation_id: "100", role: "user", body: "m4" },
    { id: "5", conversation_id: "100", role: "user", body: "m5" },
  ]);
  __setLlReply("replied");
  __seedReceipt("trace-1", "ch-1", { sender: "vis", payload: { body: "新消息" } });
  onAutoreply(JOB);
  const call = __emitted().find((e) => e.kind === "callApi");
  assert.ok(call, "should have called the LLM");
  const input = call.input;
  assert.ok(input && typeof input === "object", "callApi input should be an object");
  // context_window=3 → at most 3 messages in history
  assert.ok(input.messages.length <= 3, `history capped at 3, got ${input.messages.length}`);
});
