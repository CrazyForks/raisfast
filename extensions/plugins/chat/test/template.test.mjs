// Unit tests for lib/template.js — the `{var}` renderer shared by outbound
// payloads and contact enrichment.

import { test } from "node:test";
import assert from "node:assert/strict";

import { renderTemplate } from "../lib/template.js";

const ctx = {
  sender: "8760804080",
  reply: { chat_id: "123456789", webhook: "https://hook/x" },
  msg: { body: "hello world", id: "m-1" },
  conv: { contact_id: "c-1" },
};

test("renders dotted paths from context", () => {
  assert.equal(renderTemplate("{reply.chat_id}", ctx), "123456789");
  assert.equal(renderTemplate("{msg.body}", ctx), "hello world");
  assert.equal(renderTemplate("{conv.contact_id}", ctx), "c-1");
  assert.equal(renderTemplate("{sender}", ctx), "8760804080");
});

test("renders nested object templates", () => {
  assert.deepEqual(
    renderTemplate(
      { chat_id: "{reply.chat_id}", text: "{msg.body}" },
      ctx,
    ),
    { chat_id: "123456789", text: "hello world" },
  );
});

test("renders templates embedded in strings", () => {
  assert.equal(
    renderTemplate('{"text":"{msg.body}"}', ctx),
    '{"text":"hello world"}',
  );
});

test("missing paths render empty strings", () => {
  assert.equal(renderTemplate("{reply.open_id}", ctx), "");
  assert.equal(renderTemplate("{nope.x}", ctx), "");
});

test("renders arrays", () => {
  assert.deepEqual(renderTemplate(["{msg.id}", "{sender}"], ctx), ["m-1", "8760804080"]);
});
