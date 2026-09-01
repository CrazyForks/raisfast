// Unit tests for services/assign.js — candidate filtering, round-robin cursor
// advancement, least-busy tie-break, and assignment application.
// Presence is injected (`presenceAvailable`), so no kernel presence API needed.

import { test, beforeEach } from "node:test";
import assert from "node:assert/strict";

import {
  __reset,
  __seed,
  __rows,
  __emitted,
  __seedPresenceAvailable,
} from "./sdk-mock.mjs";
import {
  teamMembers,
  roundRobinPick,
  leastBusyPick,
  applyAssignment,
  pickAssignee,
  candidates,
  onAssign,
  onAssignScan,
} from "../services/assign.js";

beforeEach(() => __reset());

function seedTeam(teamId = "t1", assignConfig = {}) {
  __seed("chat_teams", [{ id: teamId, name: "T", allow_auto_assign: true, assign_config: assignConfig }]);
}

function seedMember(teamId, userId) {
  __seed("chat_team_members", [{ id: `m${userId}`, team_id: teamId, user_id: userId }]);
}

function seedProfile(userId, overrides = {}) {
  __seed("chat_agent_profiles", [{
    id: `p${userId}`,
    user_id: userId,
    availability: "online",
    max_open: 20,
    ...overrides,
  }]);
}

function seedConv(overrides = {}) {
  __seed("chat_conversations", [{
    id: "c1",
    contact_id: "50",
    status: "open",
    assignee_id: null,
    bot_status: "disabled",
    ...overrides,
  }]);
}

test("roundRobinPick cycles through candidates and advances the cursor", () => {
  const ids = ["3", "1", "2"];
  const first = roundRobinPick("t1", ids, null);
  assert.equal(first.userId, "1"); // sorted → first
  const second = roundRobinPick("t1", ids, first.cursor);
  assert.equal(second.userId, "2");
  const third = roundRobinPick("t1", ids, second.cursor);
  assert.equal(third.userId, "3");
  const wrap = roundRobinPick("t1", ids, third.cursor);
  assert.equal(wrap.userId, "1"); // wraps
});

test("roundRobinPick handles a cursor that is no longer a candidate", () => {
  const ids = ["1", "2"];
  const pick = roundRobinPick("t1", ids, "99"); // stale cursor
  assert.equal(pick.userId, "1"); // restarts from head
});

test("leastBusyPick chooses the fewest-open agent, tie → lowest id", () => {
  const pick = leastBusyPick("t1", ["2", "1"], { 1: 3, 2: 1 });
  assert.equal(pick.userId, "2");
  const tie = leastBusyPick("t1", ["2", "1"], { 1: 1, 2: 1 });
  assert.equal(tie.userId, "1");
});

test("candidates filters out offline/away and over-capacity agents", () => {
  seedMember("t1", "1");
  seedMember("t1", "2");
  seedMember("t1", "3");
  seedProfile("1", { availability: "online" });
  seedProfile("2", { availability: "away" });      // away → excluded
  seedProfile("3", { availability: "busy", max_open: 1 });
  // agent 3 is at capacity (1 open >= max_open=1)
  __seed("chat_conversations", [
    { id: "x1", assignee_id: "3", status: "open" },
  ]);
  const avail = ["1", "2", "3"];
  const got = candidates("t1", avail, { defaultMaxOpen: 20 });
  assert.deepEqual(got.sort(), ["1"]);
});

test("pickAssignee round_robin assigns and advances the team cursor", () => {
  seedTeam("t1", {});
  seedMember("t1", "1");
  seedMember("t1", "2");
  seedProfile("1", { availability: "online" });
  seedProfile("2", { availability: "online" });
  seedConv();
  const who = pickAssignee({ id: "c1", team_id: "t1" }, ["1", "2"], {});
  assert.ok(["1", "2"].includes(who), `assigned to ${who}`);
  // cursor advanced on the team row
  const team = __rows("chat_teams")[0];
  assert.equal(team.assign_config.cursor, who);
});

test("applyAssignment sets assignee + writes activity + emits", () => {
  seedConv();
  const uid = applyAssignment("c1", "42", { reason: "auto_assign" });
  assert.equal(uid, "42");
  const conv = __rows("chat_conversations")[0];
  assert.equal(conv.assignee_id, "42");
  const msg = __rows("chat_messages")[0];
  assert.equal(msg.role, "activity");
  const events = __emitted().filter((e) => e.kind === "event" && e.type === "chat.assignment");
  assert.equal(events.length, 1);
  assert.equal(events[0].data.assignee_id, "42");
});

test("pickAssignee returns null when no one is available", () => {
  seedTeam("t1", {});
  seedMember("t1", "1");
  seedProfile("1", { availability: "online" });
  seedConv();
  const who = pickAssignee({ id: "c1", team_id: "t1" }, [], {});
  assert.equal(who, null);
  // conversation stays unassigned
  assert.equal(__rows("chat_conversations")[0].assignee_id, null);
});

test("onAssign assigns via presenceAvailable + round-robin, skips already-assigned", () => {
  seedTeam("t1", {});
  seedMember("t1", "1");
  seedMember("t1", "2");
  seedProfile("1", { availability: "online" });
  seedProfile("2", { availability: "online" });
  seedConv();
  __seedPresenceAvailable(["1", "2"]);
  const out = onAssign({ conversation_id: "c1", tenant_id: "default" });
  assert.ok(["1", "2"].includes(out.assigned_to), `assigned ${out.assigned_to}`);
  assert.equal(__rows("chat_conversations")[0].assignee_id, out.assigned_to);
  // second run: already assigned → coalesced
  const again = onAssign({ conversation_id: "c1", tenant_id: "default" });
  assert.equal(again.skipped, "already_assigned");
});

test("onAssign coalesces when presence has no candidates", () => {
  seedTeam("t1", {});
  seedMember("t1", "1");
  seedProfile("1", { availability: "online" });
  seedConv();
  __seedPresenceAvailable([]); // nobody online
  const out = onAssign({ conversation_id: "c1", tenant_id: "default" });
  assert.equal(out.skipped, "no_candidate");
  assert.equal(__rows("chat_conversations")[0].assignee_id, null);
});

test("onAssignScan assigns all eligible unassigned conversations", () => {
  seedTeam("t1", {});
  seedMember("t1", "1");
  seedMember("t1", "2");
  seedProfile("1", { availability: "online" });
  seedProfile("2", { availability: "online" });
  // two unassigned open conversations
  seedConv({ id: "c1" });
  seedConv({ id: "c2" });
  __seedPresenceAvailable(["1", "2"]);
  const out = onAssignScan({ tenant_id: "default" });
  assert.equal(out.assigned, 2);
  assert.equal(out.skipped, 0);
  const rows = __rows("chat_conversations");
  assert.ok(rows.every((r) => r.assignee_id != null), "both assigned");
});

test("onAssignScan coalesces already-assigned and no-candidate", () => {
  seedTeam("t1", {});
  seedMember("t1", "1");
  seedProfile("1", { availability: "online" });
  seedConv({ id: "c1", assignee_id: "99" }); // already assigned
  seedConv({ id: "c2" });                      // no presence → no candidate
  __seedPresenceAvailable([]);
  const out = onAssignScan({ tenant_id: "default" });
  assert.equal(out.assigned, 0);
  assert.equal(out.skipped, 1);
  assert.equal(out.no_candidate, 1);
  assert.equal(out.scanned, 2);
});
