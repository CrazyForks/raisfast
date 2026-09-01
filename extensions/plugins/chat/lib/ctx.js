// Shared plugin context: content-type names, id/timestamp helpers, and
// input parsing utilities. Every module in the chat plugin imports from here
// instead of re-declaring the same constants/functions in each file.

import {
    ctFind,
} from 'sdk';

// ── Content-type references ──────────────────────────────────
// Group-aware plural names (`group/plural`). These are the only stable
// handles; bare plural names resolve too but the explicit group form is the
// documented convention (architecture §3, plugin boundary contract).

export const CT_MSG = 'chat/chat_messages';
export const CT_CONV = 'chat/chat_conversations';
export const CT_CONTACT = 'chat/chat_contacts';
export const CT_IDENTITY = 'chat/chat_contact_identities';
export const CT_INBOX = 'chat/chat_inboxes';
export const CT_BOT = 'chat/chat_bots';
export const CT_TEAM = 'chat/chat_teams';
export const CT_TEAM_MEMBER = 'chat/chat_team_members';
export const CT_AGENT_PROFILE = 'chat/chat_agent_profiles';

// ── Time ─────────────────────────────────────────────────────

export const nowISO = () => new Date().toISOString();

// ── Id / row helpers ─────────────────────────────────────────

// Ids cross the plugin boundary as strings (snowflake ids exceed the JS
// safe-integer range); keep them strings — filters and host APIs coerce.
export function idOf(row) {
    return row && row.id != null ? String(row.id) : null;
}

export function findFirst(ct, filters, sort = 'id desc') {
    const res = ctFind(ct, { filters, page_size: 1, sort });
    return res.rows?.[0] ?? null;
}

// ── Input parsing ────────────────────────────────────────────

// Job payloads are a JSON string (or object) carrying trace_id/channel_key
// plus job-specific fields.
export function parseJobInput(input) {
    if (typeof input === 'string') return JSON.parse(input);
    return input ?? {};
}

// Route input is a JSON string:
//   { path, method, body:<string>, headers, params:{id}, query:{...},
//     auth:{ user_id, role, roles, tenant_id } }
export function routeInput(input) {
    const data = typeof input === 'string' ? JSON.parse(input) : (input ?? {});
    return data;
}

export function jsonBody(input) {
    const raw = input && typeof input.body === 'string' ? input.body : '';
    if (!raw) return {};
    try { return JSON.parse(raw); } catch { return {}; }
}

export function pathParam(input, name) {
    return input?.params?.[name] ?? null;
}
