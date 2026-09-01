// Unit tests for services/inbox.js — conversation/identity lifecycle.
// These run with the `sdk` mock (no kernel/DB): seed CT rows, call the pure
// logic, assert on created rows + emitted events.

import { test, beforeEach } from "node:test";
import assert from "node:assert/strict";

import {
  __reset,
  __seed,
  __rows,
  __emitted,
} from "./sdk-mock.mjs";
import {
  ensureConversation,
  ensureIdentity,
  linkMessage,
} from "../services/inbox.js";

beforeEach(() => __reset());

test("ensureIdentity reuses the existing (channel,sender) contact", () => {
  __seed("chat_contact_identities", [
    { id: "10", contact_id: "100", channel: "web", sender: "vis-1" },
  ]);
  const contactId = ensureIdentity("web", "vis-1");
  assert.equal(contactId, "100");
  // no new contact created
  assert.equal(__rows("chat_contacts").length, 0);
});

test("ensureIdentity creates contact + identity on first sight", () => {
  const contactId = ensureIdentity("web", "vis-new");
  assert.ok(contactId);
  const contacts = __rows("chat_contacts");
  const identities = __rows("chat_contact_identities");
  assert.equal(contacts.length, 1);
  assert.equal(identities.length, 1);
  assert.equal(identities[0].sender, "vis-new");
  assert.equal(identities[0].contact_id, contacts[0].id);
});

test("ensureConversation reuses an open conversation for the same contact", () => {
  __seed("chat_conversations", [
    { id: "5", contact_id: "100", status: "open", bot_status: "disabled" },
  ]);
  const conv = ensureConversation("100", null, false);
  assert.equal(conv.id, "5");
  assert.equal(conv.isNew, false);
  assert.equal(__rows("chat_conversations").length, 1);
});

test("ensureConversation reopens the latest resolved conversation when reopen_enabled", () => {
  __seed("chat_conversations", [
    { id: "9", contact_id: "100", status: "resolved", bot_status: "disabled", reopened_count: 0 },
  ]);
  const conv = ensureConversation("100", { reopen_enabled: true }, false);
  assert.equal(conv.id, "9");
  assert.equal(conv.isNew, false);
  const row = __rows("chat_conversations")[0];
  assert.equal(row.status, "open");
  assert.equal(row.reopened_count, 1);
});

test("ensureConversation creates a new human-first conversation when no live one", () => {
  const conv = ensureConversation("100", null, false);
  assert.equal(conv.isNew, true);
  const row = __rows("chat_conversations")[0];
  assert.equal(row.status, "open");
  assert.equal(row.bot_status, "disabled"); // no bot = human-only
});

test("ensureConversation bot-bound conversation starts pending+active", () => {
  const conv = ensureConversation("100", null, true);
  assert.equal(conv.isNew, true);
  const row = __rows("chat_conversations")[0];
  assert.equal(row.status, "pending");
  assert.equal(row.bot_status, "active");
});

test("linkMessage attaches conversation_id to the routed raw message", () => {
  __seed("chat_messages", [
    { id: "20", receipt_id: "trace-1", conversation_id: null },
  ]);
  const msgId = linkMessage("trace-1", null, "999", false);
  assert.equal(msgId, "20");
  assert.equal(__rows("chat_messages")[0].conversation_id, "999");
});

test("linkMessage finds by external_id when receipt_id has no message", () => {
  __seed("chat_messages", [
    { id: "21", receipt_id: "other", external_id: "ext-9", conversation_id: null },
  ]);
  const msgId = linkMessage("trace-1", "ext-9", "999", false);
  assert.equal(msgId, "21");
  assert.equal(__rows("chat_messages")[0].conversation_id, "999");
});
