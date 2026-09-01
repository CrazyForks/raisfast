// Workspace conversation routes (CH-1). Action endpoints with side effects
// (send/assign/status/bot/labels/read) — declared in manifest [[routes]],
// handlers are synchronous (QuickJS does not await returned promises).

import {
    ctCreate,
    ctFind,
    ctGet,
    ctUpdate,
    jobEnqueue,
} from 'sdk';
import {
    CT_CONTACT,
    CT_CONV,
    CT_MSG,
    idOf,
    findFirst,
    jsonBody,
    nowISO,
    pathParam,
    routeInput,
} from '../lib/ctx.js';
import { agentId, visibleRows } from '../lib/auth.js';
import { fail, invalid, notFound } from '../lib/errors.js';
import {
    emitAssignment,
    emitBotToggled,
    emitConversationUpdated,
    emitMessageCreated,
} from '../lib/events.js';

// Decorate conversation rows with their contact's name/email/avatar. PK ids
// cross the plugin boundary base62-encoded (encode_id) while plain bigint
// fields (contact_id) come back as digit strings — so a batch `id in [...]`
// lookup can't key the map by contact_id. ctGet accepts either form
// (field_bind decodes), so fetch per unique contact.
function denormalizeContacts(convs) {
    const ids = [...new Set(
        convs
            .map((c) => c.contact_id != null ? String(c.contact_id) : null)
            .filter(Boolean),
    )];
    const map = {};
    for (const id of ids) {
        const row = ctGet(CT_CONTACT, id);
        if (row) map[id] = row;
    }
    return (row) => {
        const c = row.contact_id != null ? map[String(row.contact_id)] : null;
        return {
            ...row,
            contact_name: c?.name ?? null,
            contact_email: c?.email ?? null,
            contact_avatar_url: c?.avatar_url ?? null,
        };
    };
}

// GET /api/v1/plugins/chat/conversations
export function listConversations(input) {
    const data = routeInput(input);
    const q = data.query ?? {};
    const filters = [];
    if (q.status) filters.push({ field: 'status', value: String(q.status) });
    if (q.inbox) filters.push({ field: 'inbox_id', value: String(q.inbox) });
    if (q.priority) filters.push({ field: 'priority', value: String(q.priority) });
    if (q.bot === 'true') filters.push({ field: 'bot_status', value: 'active' });

    const sort = String(q.sort ?? 'last_message_at desc');
    const page = Math.max(Number(q.page ?? 1), 1);
    const pageSize = Math.min(Math.max(Number(q.page_size ?? 50), 1), 100);

    // Fetch a generous window; assignee visibility + label/keyword filtering
    // happen in JS (CT where-DSL has no OR across assignee/None, and `LIKE`
    // on a JSON column is not portable).
    const res = ctFind(CT_CONV, { filters, sort, page_size: Math.max(page * pageSize, 200) });
    let rows = res.rows ?? [];
    rows = visibleRows(rows, data.auth);
    if (q.label) {
        const needle = String(q.label).toLowerCase();
        rows = rows.filter((r) =>
            Array.isArray(r.labels) && r.labels.some((l) => String(l).toLowerCase() === needle),
        );
    }
    if (q.q) {
        const needle = String(q.q).toLowerCase();
        rows = rows.filter((r) =>
            (r.id ?? '').toLowerCase().includes(needle) ||
            (r.labels && JSON.stringify(r.labels).toLowerCase().includes(needle)),
        );
    }
    const total = rows.length;
    const pageRows = rows.slice((page - 1) * pageSize, page * pageSize);
    const decorate = denormalizeContacts(pageRows);
    return { items: pageRows.map(decorate), total, page, page_size: pageSize };
}

// GET /api/v1/plugins/chat/conversations/:id
export function getConversation(input) {
    const data = routeInput(input);
    const id = pathParam(data, 'id');
    if (!id) return invalid('missing id');
    const conv = ctGet(CT_CONV, id);
    if (!conv) return notFound(`conversation ${id}`);
    const visible = visibleRows([conv], data.auth);
    if (visible.length === 0) return fail(403, 'forbidden');
    const decorate = denormalizeContacts(visible);
    return decorate(conv);
}

// GET /api/v1/plugins/chat/conversations/:id/messages
export function listMessages(input) {
    const data = routeInput(input);
    const id = pathParam(data, 'id');
    if (!id) return invalid('missing id');
    const afterId = data.query?.after_id;
    const limit = Math.min(Math.max(Number(data.query?.limit ?? 50), 1), 100);

    const filters = [{ field: 'conversation_id', value: String(id) }];
    if (afterId) filters.push({ field: 'id', op: 'gt', value: String(afterId) });

    const res = ctFind(CT_MSG, { filters, sort: 'id asc', page_size: limit });
    const items = res.rows ?? [];
    const nextAfterId = items.length >= limit ? idOf(items[items.length - 1]) : null;
    return { items, next_after_id: nextAfterId };
}

// POST /api/v1/plugins/chat/conversations/:id/messages
export function sendMessage(input) {
    const data = routeInput(input);
    const id = pathParam(data, 'id');
    const me = agentId(data);
    if (!id) return invalid('missing id');
    const body = jsonBody(data);
    const text = typeof body.body === 'string' ? body.body.trim() : '';
    if (!text && !body.attachments) return invalid('body required');

    // Idempotency: client_id dedup (double-click guard).
    if (body.client_id) {
        const dup = findFirst(CT_MSG, [{ field: 'client_id', value: String(body.client_id) }]);
        if (dup) return dup;
    }

    const conv = ctGet(CT_CONV, id);
    if (!conv) return notFound(`conversation ${id}`);

    const row = {
        conversation_id: String(id),
        role: 'agent',
        content_type: body.attachments ? 'file' : 'text',
        body: text,
        private: body.private === true,
        attachments: body.attachments ?? null,
        sender_agent_id: me,
    };
    if (body.client_id) row.client_id = String(body.client_id);

    const created = ctCreate(CT_MSG, row);

    // Denormalized conversation touch: last message + status flip (pending → open).
    const upd = { last_message_at: nowISO(), last_message_role: 'agent' };
    if (conv.status === 'pending') upd.status = 'open';
    ctUpdate(CT_CONV, id, upd);

    emitMessageCreated({
        conversation_id: id,
        contact_id: conv.contact_id,
        message_id: idOf(created),
        role: 'agent',
        body: text,
        private: row.private,
    });

    // Outbound dispatch (SSE already pushed; the job marks delivered / calls
    // the configured api-client for IM/email channels).
    jobEnqueue('chat.egress', { message_id: idOf(created) }, { max_attempts: 3 });
    return created;
}

// POST /api/v1/plugins/chat/conversations/:id/assign
export function assignConversation(input) {
    const data = routeInput(input);
    const id = pathParam(data, 'id');
    const body = jsonBody(data);
    if (!id || !body.agent_id) return invalid('agent_id required');
    if (!ctGet(CT_CONV, id)) return notFound(`conversation ${id}`);
    const updated = ctUpdate(CT_CONV, id, { assignee_id: String(body.agent_id) });
    emitAssignment({ conversation_id: id, assignee_id: String(body.agent_id) });
    return updated;
}

// POST /api/v1/plugins/chat/conversations/:id/unassign
export function unassignConversation(input) {
    const data = routeInput(input);
    const id = pathParam(data, 'id');
    if (!id) return invalid('missing id');
    if (!ctGet(CT_CONV, id)) return notFound(`conversation ${id}`);
    const updated = ctUpdate(CT_CONV, id, { assignee_id: null });
    emitAssignment({ conversation_id: id, assignee_id: null });
    return updated;
}

// POST /api/v1/plugins/chat/conversations/:id/team
export function assignTeam(input) {
    const data = routeInput(input);
    const id = pathParam(data, 'id');
    const body = jsonBody(data);
    if (!id) return invalid('missing id');
    if (!ctGet(CT_CONV, id)) return notFound(`conversation ${id}`);
    const updated = ctUpdate(CT_CONV, id, { team_id: body.team_id ? String(body.team_id) : null });
    emitConversationUpdated({ conversation_id: id, ...updated });
    return updated;
}

// POST /api/v1/plugins/chat/conversations/:id/status
export function updateStatus(input) {
    const data = routeInput(input);
    const id = pathParam(data, 'id');
    const body = jsonBody(data);
    if (!id) return invalid('missing id');
    const status = String(body.status ?? '');
    const allowed = ['open', 'pending', 'snoozed', 'resolved'];
    if (!allowed.includes(status)) return invalid(`status must be ${allowed.join('/')}`);

    const conv = ctGet(CT_CONV, id);
    if (!conv) return notFound(`conversation ${id}`);

    const upd = { status };
    if (status === 'resolved') {
        upd.resolved_at = nowISO();
        if (conv.first_response_at) {
            upd.resolution_secs = Math.max(0, Math.floor((Date.parse(upd.resolved_at) - Date.parse(conv.first_response_at)) / 1000));
        }
    } else {
        upd.resolved_at = null;
    }
    if (status === 'snoozed') {
        upd.snoozed_until = body.snoozed_until ?? null;
    } else {
        upd.snoozed_until = null;
    }

    const updated = ctUpdate(CT_CONV, id, upd);
    emitConversationUpdated({ conversation_id: id, status, ...updated });
    return updated;
}

// POST /api/v1/plugins/chat/conversations/:id/bot
export function toggleBot(input) {
    const data = routeInput(input);
    const id = pathParam(data, 'id');
    const body = jsonBody(data);
    if (!id) return invalid('missing id');
    if (!ctGet(CT_CONV, id)) return notFound(`conversation ${id}`);

    const active = body.active === true;
    // Re-enable → back to the bot queue (pending); disable → hand off to humans (open).
    const upd = active
        ? { bot_status: 'active', status: 'pending' }
        : { bot_status: 'disabled', status: 'open' };
    const updated = ctUpdate(CT_CONV, id, upd);
    emitBotToggled({ conversation_id: id, active });
    return updated;
}

// POST /api/v1/plugins/chat/conversations/:id/labels
export function updateLabels(input) {
    const data = routeInput(input);
    const id = pathParam(data, 'id');
    const body = jsonBody(data);
    if (!id) return invalid('missing id');
    const labels = Array.isArray(body.labels) ? body.labels.map(String) : [];
    if (!ctGet(CT_CONV, id)) return notFound(`conversation ${id}`);
    const updated = ctUpdate(CT_CONV, id, { labels });
    emitConversationUpdated({ conversation_id: id, labels });
    return updated;
}

// POST /api/v1/plugins/chat/conversations/:id/read
export function markConversationRead(input) {
    const data = routeInput(input);
    const id = pathParam(data, 'id');
    if (!id) return invalid('missing id');
    if (!ctGet(CT_CONV, id)) return notFound(`conversation ${id}`);
    ctUpdate(CT_CONV, id, { unread_count: 0 });
    return { ok: true };
}
