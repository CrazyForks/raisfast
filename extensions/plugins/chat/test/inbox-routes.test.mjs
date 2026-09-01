// Unit tests for routes/inbox.js — the inbox channel wizard
// (channel-app-ownership.md §5.2). Runs on the `sdk` mock: channel host API
// rows are seeded/created in-memory, and chat_inboxes ride the CT mock.

import { test, beforeEach } from "node:test";
import assert from "node:assert/strict";

import {
  __reset,
  __seed,
  __seedChannelRow,
} from "./sdk-mock.mjs";
import {
  listInboxChannels,
  createInboxChannel,
  updateInboxChannel,
} from "../routes/inbox.js";

// Route handlers receive a JSON input envelope; body/params mirror the host.
function input(body, params) {
  return JSON.stringify({ body: JSON.stringify(body ?? {}), params: params ?? {} });
}

beforeEach(() => __reset());

test("listInboxChannels joins channels with their bound inbox", () => {
  __seedChannelRow({
    id: "ch-1",
    channel_key: "chat-widget-1",
    provider: "widget",
    display_name: "Widget",
    enabled: true,
  });
  __seed("chat_inboxes", [
    { id: "i-1", channel_id: "ch-1", name: "Main", greeting: null, auto_assignment: false },
  ]);

  const res = listInboxChannels(JSON.stringify({}));
  assert.equal(res.items.length, 1);
  assert.equal(res.items[0].id, "ch-1");
  assert.equal(res.items[0].inbox.name, "Main");
});

test("listInboxChannels marks unbound channels inbox=null", () => {
  __seedChannelRow({ id: "ch-1", channel_key: "chat-widget-1", provider: "widget" });
  const res = listInboxChannels(JSON.stringify({}));
  assert.equal(res.items[0].inbox, null);
});

test("createInboxChannel creates app-owned channel + inbox atomically", () => {
  const res = createInboxChannel(
    input({ inbox: { name: "Main", greeting: "Hi", auto_assignment: true } }),
  );

  assert.ok(res.channel.id, "channel created");
  assert.equal(res.channel.app_id, "chat", "app_id host-derived, not from payload");
  assert.equal(res.channel.verify_kind, "jwt-widget", "widget template applied");
  assert.equal(res.channel.channel_key.startsWith("chat-widget-"), true);
  assert.equal(res.inbox.name, "Main");
  assert.equal(res.inbox.greeting, "Hi");
  assert.equal(res.inbox.auto_assignment, true);
  assert.equal(res.inbox.channel_id, res.channel.id, "inbox bound to channel");
});

test("createInboxChannel rejects missing inbox name", () => {
  const res = createInboxChannel(input({ inbox: {} }));
  assert.equal(res.__plugin_error, true);
  assert.equal(res.__status, 400);
});

test("updateInboxChannel patches channel + inbox fields", () => {
  __seedChannelRow({
    id: "ch-1",
    channel_key: "chat-widget-1",
    display_name: "Widget",
    enabled: true,
  });
  __seed("chat_inboxes", [
    { id: "i-1", channel_id: "ch-1", name: "Old", greeting: null, auto_assignment: false },
  ]);

  const res = updateInboxChannel(
    input(
      { channel: { display_name: "Widget 2", enabled: false }, inbox: { name: "New", auto_assignment: true } },
      { id: "ch-1" },
    ),
  );

  assert.equal(res.channel.display_name, "Widget 2");
  assert.equal(res.channel.enabled, false);
  assert.equal(res.inbox.name, "New");
  assert.equal(res.inbox.auto_assignment, true);
});
