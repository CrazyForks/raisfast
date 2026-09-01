// Unit tests for chat.ingress reply-target capture + chat.egress per-channel
// outbound (feishu callApi / dingtalk webhook) — the "IM ↔ workspace" glue.

import { test, beforeEach } from "node:test";
import assert from "node:assert/strict";

import {
  __reset,
  __seed,
  __rows,
  __emitted,
  __seedReceipt,
  __setLlReply,
} from "./sdk-mock.mjs";
import { onIngress, buildReplyTo, enrichContact } from "../services/ingress.js";
import { onEgress } from "../services/egress.js";

beforeEach(() => __reset());

// ── buildReplyTo ────────────────────────────────────────────────────

test("buildReplyTo maps feishu chat_id", () => {
  const reply = buildReplyTo("feishu", { payload: { reply_chat_id: "oc_demo" } });
  assert.deepEqual(reply, { channel: "feishu", chat_id: "oc_demo" });
});

test("buildReplyTo maps dingtalk webhook", () => {
  const reply = buildReplyTo("dingtalk", { payload: { reply_webhook: "https://oapi.dingtalk.com/..." } });
  assert.deepEqual(reply, { channel: "dingtalk", webhook: "https://oapi.dingtalk.com/..." });
});

test("buildReplyTo returns null when no reply target", () => {
  assert.equal(buildReplyTo("chat-widget", { payload: { body: "hi" } }), null);
});

test("buildReplyTo captures QQ group target", () => {
  const reply = buildReplyTo("onebot", {
    payload: { body: "hi", reply_chat_id: "123", reply_group_id: "456", reply_message_type: "group" },
  });
  assert.deepEqual(reply, { channel: "onebot", chat_id: "123", group_id: "456", message_type: "group" });
});

// ── enrichContact: config-driven profile lookup ────────────────────

const FEISHU_ENRICH = {
  client: "feishu",
  op: "get_user",
  input: { user_id: "{sender}" },
  name: "name",
  avatar: "avatar_url",
};

const TG_ENRICH = {
  client: "telegram",
  op: "get_chat",
  input: { chat_id: "{sender}" },
  name: { join: ["first_name", "last_name"], fallback: "username" },
};

test("enrichContact resolves feishu open_id to a real name", () => {
  __seed("chat_contacts", [{ id: "100", name: "ou_xxx" }]);
  __setLlReply({ name: "张三", avatar_url: "https://img/1" });
  enrichContact("feishu", "ou_xxx", "100", { enrich: FEISHU_ENRICH });
  const c = __rows("chat_contacts")[0];
  assert.equal(c.name, "张三");
  assert.equal(c.avatar_url, "https://img/1");
});

test("enrichContact skips already-resolved contacts", () => {
  __seed("chat_contacts", [{ id: "100", name: "李四" }]);
  enrichContact("feishu", "ou_xxx", "100", { enrich: FEISHU_ENRICH });
  assert.equal(__rows("chat_contacts")[0].name, "李四");
  assert.equal(__emitted().filter((e) => e.kind === "callApi").length, 0);
});

test("enrichContact keeps sender id when provider has no name", () => {
  __seed("chat_contacts", [{ id: "100", name: "ou_xxx" }]);
  __setLlReply({ error: "user not found" });
  enrichContact("feishu", "ou_xxx", "100", { enrich: FEISHU_ENRICH });
  assert.equal(__rows("chat_contacts")[0].name, "ou_xxx");
});

test("enrichContact resolves telegram from.id to first+last name", () => {
  __seed("chat_contacts", [{ id: "100", name: "123456789" }]);
  __setLlReply({ first_name: "张三", last_name: "测试" });
  enrichContact("telegram", "123456789", "100", { enrich: TG_ENRICH });
  assert.equal(__rows("chat_contacts")[0].name, "张三 测试");
});

test("enrichContact falls back to username when no first/last name", () => {
  __seed("chat_contacts", [{ id: "100", name: "123456789" }]);
  __setLlReply({ username: "zhangsan" });
  enrichContact("telegram", "123456789", "100", { enrich: TG_ENRICH });
  assert.equal(__rows("chat_contacts")[0].name, "zhangsan");
});

test("enrichContact does nothing without enrich config", () => {
  __seed("chat_contacts", [{ id: "100", name: "ou_xxx" }]);
  enrichContact("feishu", "ou_xxx", "100", {});
  assert.equal(__rows("chat_contacts")[0].name, "ou_xxx");
  assert.equal(__emitted().filter((e) => e.kind === "callApi").length, 0);
});

// ── chat.ingress: persists reply_to on the conversation ─────────────

test("onIngress captures feishu reply_to onto the conversation", () => {
  __seedReceipt("trace-1", "ch-9", {
    sender: "ou_openid_1",
    external_id: "evt-1",
    payload: { body: "你好", reply_chat_id: "oc_chat_1" },
  });
  // The integration pipeline routes the envelope into chat_messages first.
  __seed("chat_messages", [
    { id: "m-1", receipt_id: "trace-1", external_id: "evt-1", body: "你好", role: "user", conversation_id: null },
  ]);
  const res = onIngress(JSON.stringify({ trace_id: "trace-1", channel_key: "feishu" }));
  assert.ok(res.conversation_id);
  const conv = __rows("chat_conversations")[0];
  assert.equal(conv.contact_id, __rows("chat_contacts")[0].id);
  assert.deepEqual(conv.reply_to, { channel: "feishu", chat_id: "oc_chat_1" });
  // message linked + role=user
  const msg = __rows("chat_messages")[0];
  assert.equal(msg.conversation_id, conv.id);
});

test("onIngress updates reply_to on an existing conversation (dingtalk webhook per message)", () => {
  __seedReceipt("trace-1", "ch-9", {
    sender: "staff-1",
    external_id: "m-1",
    payload: { body: "hi", reply_webhook: "https://hook/1" },
  });
  __seed("chat_contacts", [{ id: "100", name: "staff-1" }]);
  __seed("chat_contact_identities", [{ id: "i1", contact_id: "100", channel: "dingtalk", sender: "staff-1" }]);
  __seed("chat_conversations", [
    { id: "200", contact_id: "100", status: "open", bot_status: "disabled", reply_to: { channel: "dingtalk", webhook: "https://hook/old" } },
  ]);

  const res = onIngress(JSON.stringify({ trace_id: "trace-1", channel_key: "dingtalk" }));
  const conv = __rows("chat_conversations").find((c) => c.id === "200");
  assert.deepEqual(conv.reply_to, { channel: "dingtalk", webhook: "https://hook/1" });
});

// ── chat.egress: per-channel dispatch ───────────────────────────────

function seedConv(overrides) {
  __seed("chat_inboxes", [
    { id: "in-1", channel_id: "ch-9", name: "IM", egress: overrides.egress },
  ]);
  __seed("chat_conversations", [
    { id: "conv-1", inbox_id: "in-1", reply_to: overrides.reply_to, status: "open" },
  ]);
  __seed("chat_messages", [
    { id: "msg-1", conversation_id: "conv-1", role: "agent", body: "hello im", status: "sent" },
  ]);
}

test("onEgress dingtalk → httpPost sessionWebhook", () => {
  seedConv({
    egress: { kind: "webhook" },
    reply_to: { channel: "dingtalk", webhook: "https://oapi.dingtalk.com/robot/send?access_token=x" },
  });
  const res = onEgress(JSON.stringify({ message_id: "msg-1" }));
  assert.equal(res.status, "delivered");
  assert.equal(res.kind, "webhook");
  const calls = __emitted().filter((e) => e.kind === "httpPost");
  assert.equal(calls.length, 1);
  assert.equal(calls[0].url, "https://oapi.dingtalk.com/robot/send?access_token=x");
  const body = JSON.parse(calls[0].body);
  assert.equal(body.msgtype, "text");
  assert.equal(body.text.content, "hello im");
  assert.equal(__rows("chat_messages")[0].status, "delivered");
});

test("onEgress feishu → renders egress.input template into callApi", () => {
  seedConv({
    egress: {
      kind: "api",
      client: "feishu",
      op: "send_text",
      input: {
        receive_id: "{reply.chat_id}",
        receive_id_type: "chat_id",
        msg_type: "text",
        content: '{"text":"{msg.body}"}',
      },
    },
    reply_to: { channel: "feishu", chat_id: "oc_chat_1" },
  });
  const res = onEgress(JSON.stringify({ message_id: "msg-1" }));
  assert.equal(res.kind, "api");
  const calls = __emitted().filter((e) => e.kind === "callApi");
  assert.equal(calls.length, 1);
  assert.equal(calls[0].client, "feishu");
  assert.equal(calls[0].op, "send_text");
  assert.equal(calls[0].input.receive_id, "oc_chat_1");
  assert.equal(calls[0].input.receive_id_type, "chat_id");
  assert.equal(JSON.parse(calls[0].input.content).text, "hello im");
});

test("onEgress telegram → renders {chat_id, text} template into callApi", () => {
  seedConv({
    egress: {
      kind: "api",
      client: "telegram",
      op: "send_text",
      input: { chat_id: "{reply.chat_id}", text: "{msg.body}" },
    },
    reply_to: { channel: "telegram", chat_id: "123456789" },
  });
  const res = onEgress(JSON.stringify({ message_id: "msg-1" }));
  assert.equal(res.kind, "api");
  const calls = __emitted().filter((e) => e.kind === "callApi");
  assert.equal(calls.length, 1);
  assert.equal(calls[0].client, "telegram");
  assert.deepEqual(calls[0].input, { chat_id: "123456789", text: "hello im" });
});

test("onEgress QQ group → prunes empty group_id, sends group target", () => {
  seedConv({
    egress: {
      kind: "api",
      client: "onebot",
      op: "send_msg",
      input: {
        message_type: "{reply.message_type}",
        user_id: "{reply.chat_id}",
        group_id: "{reply.group_id}",
        message: "{msg.body}",
      },
    },
    reply_to: { channel: "onebot", chat_id: "123", group_id: "456", message_type: "group" },
  });
  const res = onEgress(JSON.stringify({ message_id: "msg-1" }));
  assert.equal(res.kind, "api");
  const calls = __emitted().filter((e) => e.kind === "callApi");
  assert.equal(calls.length, 1);
  assert.equal(calls[0].client, "onebot");
  assert.deepEqual(calls[0].input, {
    message_type: "group",
    user_id: "123",
    group_id: "456",
    message: "hello im",
  });
});

test("onEgress QQ private → group_id omitted (empty pruned)", () => {
  seedConv({
    egress: {
      kind: "api",
      client: "onebot",
      op: "send_msg",
      input: {
        message_type: "{reply.message_type}",
        user_id: "{reply.chat_id}",
        group_id: "{reply.group_id}",
        message: "{msg.body}",
      },
    },
    reply_to: { channel: "onebot", chat_id: "123", message_type: "private" },
  });
  const res = onEgress(JSON.stringify({ message_id: "msg-1" }));
  const calls = __emitted().filter((e) => e.kind === "callApi");
  assert.deepEqual(calls[0].input, {
    message_type: "private",
    user_id: "123",
    message: "hello im",
  });
});

test("onEgress widget (no reply) → sse delivered", () => {
  seedConv({ egress: { kind: "sse" }, reply_to: null });
  const res = onEgress(JSON.stringify({ message_id: "msg-1" }));
  assert.equal(res.kind, "sse");
  assert.equal(res.status, "delivered");
  assert.equal(__emitted().filter((e) => e.kind === "callApi").length, 0);
});

test("onEgress webhook marks failed when httpPost errors", () => {
  seedConv({
    egress: { kind: "webhook" },
    reply_to: { channel: "dingtalk", webhook: "https://oapi.dingtalk.com/robot/send" },
  });
  // Make the mock httpPost return an error string (as the host does when the
  // URL is not whitelisted or the request fails).
  globalThis.__httpPostError = "error: URL not allowed";
  try {
    assert.throws(() => onEgress(JSON.stringify({ message_id: "msg-1" })));
  } finally {
    delete globalThis.__httpPostError;
  }
  assert.equal(__rows("chat_messages")[0].status, "failed");
});

test("onEgress webhook marks failed on non-2xx http status", () => {
  seedConv({
    egress: { kind: "webhook" },
    reply_to: { channel: "dingtalk", webhook: "https://oapi.dingtalk.com/robot/send" },
  });
  globalThis.__httpPostResp = '{"status":400,"body":"invalid session"}';
  try {
    assert.throws(() => onEgress(JSON.stringify({ message_id: "msg-1" })));
  } finally {
    delete globalThis.__httpPostResp;
  }
  assert.equal(__rows("chat_messages")[0].status, "failed");
});
