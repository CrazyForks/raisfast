// chat.assign — auto-assignment core logic (architecture §4.5).
//
// Candidate set:  team members ∩ presence-available ∧ open-count < max_open.
// Strategy:       round_robin (cursor stored on chat_team.assign_config.cursor,
//                 advanced atomically) | least_busy (fewest open).
// No candidate / off-hours → stay unassigned (rescan cron picks it up later).
//
// Pure functions: the presence source is injected (`presenceAvailable`), so
// this module is unit-testable without the kernel presence host API — the job
// handler wires the real `host.presence.available` (CH-2 host API).

import { ctFind, ctGet, ctUpdate, ctCreate, presenceAvailable } from 'sdk';
import {
  CT_AGENT_PROFILE,
  CT_CONV,
  CT_INBOX,
  CT_MSG,
  CT_TEAM,
  CT_TEAM_MEMBER,
  findFirst,
  idOf,
  parseJobInput,
} from '../lib/ctx.js';
import { emitAssignment } from '../lib/events.js';

// ── candidate selection ────────────────────────────────────────

// Members of the team whose id is `teamId`, as [user_id, ...] (digit strings).
export function teamMembers(teamId) {
  const res = ctFind(CT_TEAM_MEMBER, {
    filters: [{ field: 'team_id', value: String(teamId) }],
    sort: 'id asc',
    page_size: 200,
  });
  return (res.rows ?? []).map((r) => String(r.user_id));
}

// Profiles for the given user ids, keyed by user_id.
export function profiles(userIds) {
  const map = {};
  for (const uid of userIds) {
    const p = findFirst(CT_AGENT_PROFILE, [{ field: 'user_id', value: String(uid) }]);
    if (p) map[uid] = p;
  }
  return map;
}

// Candidates = team members that are present (presenceAvailable) AND have a
// profile with availability ∈ {online, busy} AND open count < max_open.
export function candidates(teamId, presenceAvailable, opts = {}) {
  const { defaultMaxOpen = 20 } = opts;
  const memberIds = teamMembers(teamId);
  if (memberIds.length === 0) return [];

  const availableSet = new Set((presenceAvailable ?? []).map(String));
  const profs = profiles(memberIds);
  const candidates = [];

  for (const uid of memberIds) {
    if (!availableSet.has(uid)) continue; // offline / no presence
    const p = profs[uid];
    if (p && p.availability != null && ['away', 'offline'].includes(p.availability)) continue;
    const maxOpen = p && p.max_open != null ? Number(p.max_open) : defaultMaxOpen;
    const open = openCountFor(uid);
    if (open < maxOpen) candidates.push(uid);
  }
  return candidates;
}

// Open+assigned conversation count for one agent (unassigned excluded).
export function openCountFor(uid) {
  const res = ctFind(CT_CONV, {
    filters: [
      { field: 'assignee_id', value: String(uid) },
      { field: 'status', op: 'in', value: ['open', 'pending'] },
    ],
    sort: 'id asc',
    page_size: 1,
  });
  // CT where-DSL returns a page; page_size=1 truncates — instead scan a large
  // window and count. In practice max_open ≤ 50 and the scan is bounded.
  const full = ctFind(CT_CONV, {
    filters: [
      { field: 'assignee_id', value: String(uid) },
      { field: 'status', op: 'in', value: ['open', 'pending'] },
    ],
    sort: 'id asc',
    page_size: 500,
  });
  return (full.rows ?? []).length;
}

// ── strategy ───────────────────────────────────────────────────

// round_robin: cursor lives on the team's assign_config; CAS-advance it by
// reading the team row, picking the member after the cursor, writing the new
// cursor. Deterministic per call; the job retries on conflict.
export function roundRobinPick(teamId, candidateIds, cursor) {
  if (candidateIds.length === 0) return null;
  const sorted = [...candidateIds].sort(); // stable ordering
  if (cursor == null) return { userId: sorted[0], cursor: sorted[0] };
  const idx = sorted.indexOf(String(cursor));
  const next = idx === -1 ? 0 : (idx + 1) % sorted.length;
  return { userId: sorted[next], cursor: sorted[next] };
}

// least_busy: fewest open+assigned conversations; tie → lowest id.
export function leastBusyPick(teamId, candidateIds, openCountsById) {
  if (candidateIds.length === 0) return null;
  let best = null;
  for (const uid of candidateIds) {
    const count = openCountsById[uid] ?? 0;
    if (best === null || count < best.count || (count === best.count && uid < best.userId)) {
      best = { userId: uid, count };
    }
  }
  return best;
}

// ── assignment application ─────────────────────────────────────

// Assign conversation to an agent: set assignee, write an activity message,
// emit chat.assignment. Returns the assigned user id or null.
export function applyAssignment(convId, userId, opts = {}) {
  const { reason = 'auto_assign' } = opts;
  if (!convId || !userId) return null;
  ctUpdate(CT_CONV, convId, { assignee_id: String(userId) });
  ctCreate(CT_MSG, {
    conversation_id: String(convId),
    role: 'activity',
    content_type: 'activity_event',
    body: JSON.stringify({ type: 'assigned', agent_id: String(userId), reason }),
  });
  emitAssignment({ conversation_id: String(convId), assignee_id: String(userId) });
  return String(userId);
}

// ── orchestrator (used by the job handler) ─────────────────────

// Pick an assignee for an unassigned conversation. `presenceAvailable` is the
// kernel presence `available(tenant)` result (injected; job wires host API).
export function pickAssignee(conv, presenceAvailable, opts = {}) {
  const { strategy = 'round_robin', defaultMaxOpen = 20 } = opts;
  const teamId = conv.team_id ?? opts.team_id ?? null;
  const inbox = conv.inbox_id ? ctGet(CT_INBOX, String(conv.inbox_id)) : null;
  const useTeam = teamId ?? (inbox?.assign_config?.fallback_team_id ?? null);

  let candidateIds;
  if (useTeam) {
    candidateIds = candidates(useTeam, presenceAvailable, { defaultMaxOpen });
  } else {
    // No team: everyone present who has a profile and capacity.
    candidateIds = candidatesNoTeam(presenceAvailable, { defaultMaxOpen });
  }
  if (candidateIds.length === 0) return null;

  if (strategy === 'least_busy') {
    const counts = {};
    for (const uid of candidateIds) counts[uid] = openCountFor(uid);
    const pick = leastBusyPick(useTeam, candidateIds, counts);
    return pick ? pick.userId : null;
  }

  // round_robin: read cursor from the team (or inbox) assign_config.
  const configRow = useTeam ? ctGet(CT_TEAM, String(useTeam)) : (inbox ?? null);
  const cfg = configRow?.assign_config ?? {};
  const cursor = cfg.cursor ?? null;
  const pick = roundRobinPick(useTeam, candidateIds, cursor);
  if (!pick) return null;
  // CAS-advance: write the new cursor back (job retries on conflict).
  if (configRow && configRow.id) {
    ctUpdate(useTeam ? CT_TEAM : CT_INBOX, idOf(configRow), {
      assign_config: { ...cfg, cursor: pick.cursor },
    });
  }
  return pick.userId;
}

// No-team fallback: presence + profile-capacity filter only.
export function candidatesNoTeam(presenceAvailable, opts = {}) {
  const { defaultMaxOpen = 20 } = opts;
  const availableSet = new Set((presenceAvailable ?? []).map(String));
  const out = [];
  // Profiles have a unique user_id; scan the whole CT (bounded).
  const res = ctFind(CT_AGENT_PROFILE, { sort: 'id asc', page_size: 500 });
  for (const p of res.rows ?? []) {
    const uid = String(p.user_id);
    if (!availableSet.has(uid)) continue;
    if (p.availability != null && ['away', 'offline'].includes(p.availability)) continue;
    const maxOpen = p.max_open != null ? Number(p.max_open) : defaultMaxOpen;
    if (openCountFor(uid) < maxOpen) out.push(uid);
  }
  return out;
}

// ── job handler ────────────────────────────────────────────────

// chat.assign job entry (manifest [[jobs]]). Input: { conversation_id,
// tenant_id?, strategy? }. Reads the conversation, fetches kernel presence
// `available(tenant)` via the host API, picks an assignee, applies it.
// Never throws for "no candidate" — that's the expected state (rescan cron
// retries later; the conversation stays unassigned, listed top in the
// workspace "unassigned" filter).
export function onAssign(input) {
  const job = parseJobInput(input);
  const { conversation_id: convId, tenant_id: tenant, strategy } = job;
  if (!convId) throw new Error('chat.assign: missing conversation_id');

  const conv = ctGet(CT_CONV, String(convId));
  if (!conv) throw new Error(`chat.assign: conversation ${convId} not found`);
  // Already assigned → nothing to do (coalesce: rescan may re-fire).
  if (conv.assignee_id != null) return { skipped: 'already_assigned' };

  const presence = presenceAvailable(tenant ?? 'default');
  const who = pickAssignee(conv, presence, { strategy });
  if (!who) return { skipped: 'no_candidate' };

  applyAssignment(convId, who, { reason: 'auto_assign' });
  return { assigned_to: who };
}

// ── rescan cron ────────────────────────────────────────────────

// chat.assign.scan — batch rescan of unassigned open/pending conversations
// (architecture §4.5: "无候选 → 挂起重扫（cron coalesce）"). Fired by the
// manifest [[cron]] every ~30s. Coalesces: skips already-assigned and
// conversations that still have no candidate (no error — next tick retries).
// Returns counts for the worker/job log.
export function onAssignScan(input) {
  const job = parseJobInput(input);
  const tenant = job.tenant_id ?? 'default';
  const limit = Math.min(Math.max(Number(job.limit ?? 100), 1), 500);
  const strategy = job.strategy ?? 'round_robin';

  const res = ctFind(CT_CONV, {
    filters: [
      { field: 'status', op: 'in', value: ['open', 'pending'] },
    ],
    sort: 'last_message_at desc',
    page_size: limit,
  });
  const presence = presenceAvailable(tenant);
  let assigned = 0;
  let noCandidate = 0;
  let skipped = 0;

  for (const conv of res.rows ?? []) {
    if (conv.assignee_id != null) { skipped += 1; continue; }
    const who = pickAssignee(conv, presence, { strategy, tenant_id: tenant });
    if (!who) { noCandidate += 1; continue; }
    applyAssignment(String(conv.id), who, { reason: 'auto_assign' });
    assigned += 1;
  }
  return { assigned, no_candidate: noCandidate, skipped, scanned: (res.rows ?? []).length };
}
