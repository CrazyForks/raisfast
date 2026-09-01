// Chat application plugin — job handlers for the support inbox.
//
// chat.ingress  (enqueued by the integration pipeline's route_extra.jobs):
//   receipt envelope -> identity merge -> conversation locate/create
//   (bot binding decides the initial state) -> message link -> denormalized
//   counters -> SSE -> bot gate (inbox.bot_id ∧ bot.enabled ∧
//   conversation.bot_status=active -> enqueue chat.autoreply).
//
// chat.autoreply (enqueued by chat.ingress):
//   bot config from chat_bots -> handoff keyword check -> fallback guard ->
//   context window -> callApi LLM -> assistant message + SSE; `first_line`
//   mode hands off after the reply; failures hand off to a human.
//
// With no bot bound to the inbox everything lands in the human queue — the
// safe default (dev-docs/apps/chat/product.md §4.5).

import {
    ctFind, ctGet, ctCreate, ctUpdate,
    jobEnqueue, getReceipt, callApi, eventEmit, logInfo, logWarn, dbQuery, dbPh,
    issueToken, verifyToken,
} from 'sdk';

const CT_MSG = 'chat/chat_messages';
const CT_CONV = 'chat/chat_conversations';
const CT_CONTACT = 'chat/chat_contacts';
const CT_IDENTITY = 'chat/chat_contact_identities';
const CT_INBOX = 'chat/chat_inboxes';
const CT_BOT = 'chat/chat_bots';

const nowISO = () => new Date().toISOString();

// ── helpers ──────────────────────────────────────────────────

// Ids cross the plugin boundary as strings (snowflake ids exceed the JS
// safe-integer range); keep them strings — filters and host APIs coerce.
function idOf(row) {
    return row && row.id != null ? String(row.id) : null;
}

function findFirst(ct, filters, sort = 'id desc') {
    const res = ctFind(ct, { filters, page_size: 1, sort });
    return res.rows?.[0] ?? null;
}

function parseJobInput(input) {
    if (typeof input === 'string') return JSON.parse(input);
    return input ?? {};
}

// ── session primitives (mirror of the retired Rust session.rs) ──

function ensureIdentity(channelKey, sender) {
    const existing = findFirst(CT_IDENTITY, [
        { field: 'channel', value: channelKey },
        { field: 'sender', value: sender },
    ]);
    if (existing) return existing.contact_id;

    const contact = ctCreate(CT_CONTACT, { name: sender });
    ctCreate(CT_IDENTITY, {
        contact_id: idOf(contact),
        channel: channelKey,
        sender,
    });
    return idOf(contact);
}

function ensureConversation(contactId, inbox, botBound) {
    const live = findFirst(CT_CONV, [
        { field: 'contact_id', value: String(contactId) },
        { field: 'status', op: 'in', value: ['open', 'pending'] },
    ]);
    if (live) return { id: idOf(live), isNew: false, row: live };

    const reopen = inbox ? inbox.reopen_enabled !== false : true;
    if (reopen) {
        const latest = findFirst(CT_CONV, [{ field: 'contact_id', value: String(contactId) }]);
        if (latest) {
            ctUpdate(CT_CONV, idOf(latest), {
                status: 'open',
                bot_status: 'disabled',
                reopened_count: (latest.reopened_count ?? 0) + 1,
            });
            return { id: idOf(latest), isNew: false, row: latest };
        }
    }

    const row = {
        contact_id: String(contactId),
        status: botBound ? 'pending' : 'open',
        bot_status: botBound ? 'active' : 'disabled',
    };
    if (inbox && idOf(inbox)) row.inbox_id = idOf(inbox);
    const created = ctCreate(CT_CONV, row);
    return { id: idOf(created), isNew: true, row: created };
}

function linkMessage(traceId, externalId, conversationId, isNew) {
    let msg = findFirst(CT_MSG, [{ field: 'receipt_id', value: String(traceId) }]);
    if (!msg && externalId) {
        msg = findFirst(CT_MSG, [{ field: 'external_id', value: externalId }]);
    }
    if (!msg) return null;
    const unlinked = msg.conversation_id == null;
    if (isNew || unlinked) {
        ctUpdate(CT_MSG, idOf(msg), { conversation_id: conversationId });
    }
    return idOf(msg);
}

function touchConversation(convId, convRow) {
    ctUpdate(CT_CONV, convId, {
        last_message_at: nowISO(),
        last_message_role: 'user',
        unread_count: (convRow?.unread_count ?? 0) + 1,
    });
}

// ── chat.ingress ─────────────────────────────────────────────

export function onIngress(input) {
    const job = parseJobInput(input);
    const { trace_id: traceId, channel_key: channelKey } = job;
    if (!traceId || !channelKey) throw new Error('chat.ingress: missing trace_id/channel_key');

    const receipt = getReceipt(traceId);
    if (!receipt || !receipt.envelope) throw new Error(`chat.ingress: receipt ${traceId} has no envelope`);
    const env = receipt.envelope;
    const sender = env.sender;
    if (!sender) throw new Error('chat.ingress: envelope has no sender (add "sender" to the channel mapping)');

    const externalId = env.external_id ?? '';
    const body = env.payload?.body ?? '';

    // Inbox + bot binding gate (default: no bot = human-only). The receipt
    // carries the itg_channels.id — chat_inboxes.channel_id references it.
    const inbox = findFirst(CT_INBOX, [
        { field: 'channel_id', value: receipt.channel_id },
    ]);
    let bot = null;
    const botId = inbox?.bot_id;
    if (botId != null && botId !== '') {
        const botRow = ctGet(CT_BOT, botId);
        if (botRow && botRow.enabled !== false) bot = botRow;
    }

    const contactId = ensureIdentity(channelKey, sender);
    const conv = ensureConversation(contactId, inbox, bot != null);
    const messageId = linkMessage(traceId, externalId, conv.id, conv.isNew);
    touchConversation(conv.id, conv.row);

    eventEmit('chat.message.created', {
        trace_id: traceId,
        channel: channelKey,
        conversation_id: conv.id,
        contact_id: contactId,
        message_id: messageId,
        role: 'user',
        body,
    });
    logInfo(`[chat] ingress merged trace=${traceId} conv=${conv.id}`);

    // Bot gate: bound ∧ enabled ∧ conversation active.
    if (bot && conv.row.bot_status === 'active') {
        const mode = bot.mode ?? 'full';
        const opts = { max_attempts: 1 };
        if (mode === 'fallback') {
            const waitMins = bot.handoff?.fallback_wait_mins ?? 5;
            opts.delay_mins = Math.min(Math.max(waitMins, 1), 1440);
        }
        jobEnqueue('chat.autoreply', {
            trace_id: traceId,
            channel_key: channelKey,
            conversation_id: conv.id,
            bot_id: idOf(bot),
        }, opts);
    }
    return { conversation_id: conv.id };
}

// ── chat.autoreply ───────────────────────────────────────────

function handoff(convId, traceId, channelKey, reason) {
    ctUpdate(CT_CONV, convId, { bot_status: 'disabled', status: 'open' });
    eventEmit('integration.autoreply_failed', {
        trace_id: traceId,
        channel: channelKey,
        conversation_id: convId,
        reason,
    });
}

export function onAutoreply(input) {
    const job = parseJobInput(input);
    const { trace_id: traceId, channel_key: channelKey, conversation_id: convId, bot_id: botId } = job;
    if (!traceId || !channelKey || !convId || !botId) {
        throw new Error('chat.autoreply: payload must come from chat.ingress');
    }

    const bot = ctGet(CT_BOT, botId);
    if (!bot) throw new Error(`chat.autoreply: bot ${botId} not found`);
    const cfg = bot.autoreply;
    if (!cfg || !cfg.client) throw new Error('chat_bot.autoreply requires "client" (api-client key)');
    const mode = bot.mode ?? 'full';
    const contextWindow = Math.min(Math.max(cfg.context_window ?? 10, 1), 100);

    const conv = ctGet(CT_CONV, convId);
    if (!conv) throw new Error(`chat.autoreply: conversation ${convId} not found`);
    if (conv.bot_status !== 'active') {
        logInfo(`[chat] autoreply skipped (bot disabled) trace=${traceId}`);
        return { skipped: 'bot_disabled' };
    }
    if (mode === 'fallback' && conv.last_message_role === 'agent') {
        logInfo(`[chat] autoreply skipped (agent took over) trace=${traceId}`);
        return { skipped: 'agent_took_over' };
    }

    const receipt = getReceipt(traceId);
    const userText = receipt?.envelope?.payload?.body ?? '';

    // Visitor explicitly asked for a human → hand off without replying.
    const keywords = bot.handoff?.keywords ?? [];
    if (keywords.some((k) => k && userText.includes(k))) {
        handoff(convId, traceId, channelKey, 'visitor_requested');
        return { handoff: true };
    }

    // Context window (recent N messages, chronological).
    const res = ctFind(CT_MSG, {
        filters: [{ field: 'conversation_id', value: String(convId) }],
        sort: 'id desc',
        page_size: contextWindow,
    });
    const history = (res.rows ?? [])
        .slice()
        .reverse()
        .map((m) => ({ role: m.role ?? 'user', content: typeof m.body === 'string' ? m.body : String(m.body ?? '') }));
    if (userText && !history.some((m) => m.content === userText)) {
        history.push({ role: 'user', content: userText });
    }
    while (history.length > contextWindow) history.shift();

    // LLM call (openai | messages request styles).
    let llmInput;
    if (cfg.input_style === 'openai') {
        const messages = [];
        if (cfg.system_prompt) messages.push({ role: 'system', content: cfg.system_prompt });
        messages.push(...history);
        llmInput = cfg.model ? { model: cfg.model, messages } : { messages };
    } else {
        llmInput = { query: userText, messages: history };
        if (cfg.system_prompt) llmInput.system = cfg.system_prompt;
    }

    let replyText = '';
    try {
        const outRaw = callApi(cfg.client, cfg.op ?? 'chat', llmInput);
        const out = typeof outRaw === 'string' ? JSON.parse(outRaw) : outRaw;
        if (out && out.error) throw new Error(out.error);
        // Host envelope: {status, output, tokens_in, tokens_out, model}.
        let v = out.output ?? null;
        if (cfg.output_field) {
            for (const seg of String(cfg.output_field).split('.')) v = v?.[seg] ?? null;
        }
        replyText = typeof v === 'string' ? v : (v != null ? String(v) : '');
    } catch (e) {
        handoff(convId, traceId, channelKey, 'llm_failed');
        throw e;
    }
    if (!replyText) {
        handoff(convId, traceId, channelKey, 'empty_reply');
        throw new Error(`chat.autoreply: empty reply (trace ${traceId})`);
    }

    const assistant = ctCreate(CT_MSG, {
        conversation_id: String(convId),
        role: 'assistant',
        body: replyText,
        external_id: `reply-${traceId}`,
        receipt_id: traceId,
    });

    if (mode === 'first_line') {
        handoff(convId, traceId, channelKey, 'first_line_done');
    }

    eventEmit('integration.message', {
        trace_id: traceId,
        channel: channelKey,
        conversation_id: convId,
        contact_id: conv.contact_id,
        message_id: idOf(assistant),
        role: 'assistant',
        body: replyText,
    });
    eventEmit('chat.message.created', {
        trace_id: traceId,
        channel: channelKey,
        conversation_id: convId,
        contact_id: conv.contact_id,
        message_id: idOf(assistant),
        role: 'assistant',
        body: replyText,
    });
    logInfo(`[chat] autoreply delivered trace=${traceId} conv=${convId}`);
    return { conversation_id: convId };
}

// ── Workspace routes (CH-1) ──────────────────────────────────
// Agent workspace action endpoints declared in manifest [[routes]]. Handlers
// are synchronous (QuickJS does not await returned promises); all host APIs
// (ctFind/ctUpdate/eventEmit/…) are synchronous. Route input is a JSON string:
//   { path, method, body:<string>, headers, params:{id}, query:{...},
//     auth:{ user_id, role, roles, tenant_id } }

function routeInput(input) {
    const data = typeof input === 'string' ? JSON.parse(input) : (input ?? {});
    return data;
}

function jsonBody(input) {
    const raw = input && typeof input.body === 'string' ? input.body : '';
    if (!raw) return {};
    try { return JSON.parse(raw); } catch { return {}; }
}

function pathParam(input, name) {
    return input?.params?.[name] ?? null;
}

function isAdmin(auth) {
    return Array.isArray(auth?.roles) && auth.roles.includes('admin');
}

function agentId(input) {
    return input?.auth?.user_id ? String(input.auth.user_id) : null;
}

function fail(status, msg) {
    return { __plugin_error: true, __status: status, __message: msg };
}

function notFound(msg) {
    return fail(404, msg ?? 'not found');
}

function invalid(msg) {
    return fail(400, msg ?? 'invalid request');
}

// Visibility policy: platform admins see all; agents see own + unassigned.
// (Team visibility lands in CH-2 with chat_team.) Filtering happens in JS
// because CT where-DSL has no OR across assignee/None.
function visibleRows(rows, auth) {
    if (isAdmin(auth)) return rows;
    const me = String(auth?.user_id ?? '');
    if (!me) return rows.filter((r) => r.assignee_id == null);
    return rows.filter((r) => r.assignee_id == null || String(r.assignee_id) === me);
}

function denormalizeContacts(convs) {
    // PK ids cross the plugin boundary base62-encoded (encode_id) while plain
    // bigint fields (contact_id) come back as digit strings — so a batch
    // `id in [...]` lookup can't key the map by contact_id. ctGet accepts
    // either form (field_bind decodes), so fetch per unique contact.
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

    eventEmit('chat.message.created', {
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
    eventEmit('chat.assignment', { conversation_id: id, assignee_id: String(body.agent_id) });
    return updated;
}

// POST /api/v1/plugins/chat/conversations/:id/unassign
export function unassignConversation(input) {
    const data = routeInput(input);
    const id = pathParam(data, 'id');
    if (!id) return invalid('missing id');
    if (!ctGet(CT_CONV, id)) return notFound(`conversation ${id}`);
    const updated = ctUpdate(CT_CONV, id, { assignee_id: null });
    eventEmit('chat.assignment', { conversation_id: id, assignee_id: null });
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
    eventEmit('chat.conversation.updated', { conversation_id: id, ...updated });
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
    eventEmit('chat.conversation.updated', { conversation_id: id, status, ...updated });
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
    eventEmit('chat.bot.toggled', { conversation_id: id, active });
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
    eventEmit('chat.conversation.updated', { conversation_id: id, labels });
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

// GET /api/v1/plugins/chat/contacts
export function listContacts(input) {
    const data = routeInput(input);
    const q = data.query ?? {};
    const page = Math.max(Number(q.page ?? 1), 1);
    const pageSize = Math.min(Math.max(Number(q.page_size ?? 50), 1), 100);
    const res = ctFind(CT_CONTACT, { sort: 'id desc', page_size: Math.max(page * pageSize, 200) });
    let rows = res.rows ?? [];
    if (q.q) {
        const needle = String(q.q).toLowerCase();
        rows = rows.filter((r) =>
            (r.name ?? '').toLowerCase().includes(needle) ||
            (r.email ?? '').toLowerCase().includes(needle) ||
            (r.phone ?? '').toLowerCase().includes(needle),
        );
    }
    const total = rows.length;
    return { items: rows.slice((page - 1) * pageSize, page * pageSize), total, page };
}

// GET /api/v1/plugins/chat/contacts/:id/timeline
export function getContactTimeline(input) {
    const data = routeInput(input);
    const id = pathParam(data, 'id');
    if (!id) return invalid('missing id');
    const contact = ctGet(CT_CONTACT, id);
    if (!contact) return notFound(`contact ${id}`);

    const identities = ctFind(CT_IDENTITY, {
        filters: [{ field: 'contact_id', value: String(id) }],
        sort: 'id asc',
        page_size: 50,
    }).rows ?? [];

    const convRes = ctFind(CT_CONV, {
        filters: [{ field: 'contact_id', value: String(id) }],
        sort: 'id desc',
        page_size: 50,
    });
    const visible = visibleRows(convRes.rows ?? [], data.auth);
    const decorate = denormalizeContacts(visible);

    return {
        contact: {
            id: contact.id,
            name: contact.name,
            email: contact.email,
            phone: contact.phone,
            avatar_url: contact.avatar_url,
            last_seen_at: contact.last_seen_at,
            created_at: contact.created_at,
        },
        identities: (identities ?? []).map((i) => ({
            id: idOf(i),
            channel: i.channel,
            sender: i.sender,
        })),
        conversations: visible.map(decorate),
    };
}

// GET /api/v1/plugins/chat/agents
export function listAgents(input) {
    const data = routeInput(input);
    // users is not a CT — read via the read-only dbQuery host API (manifest
    // declares `database = ["read:users"]`; no JOIN — extract_table_name
    // rejects them, and email lives in user_credentials).
    // Live presence comes from the kernel presence store (architecture §5.3);
    // the workspace merges presence.* events into its store. Availability
    // here stays a base shape; live status rides SSE presence events.
    let rows;
    try {
        rows = dbQuery('SELECT id, username, display_name, avatar FROM users ORDER BY username');
    } catch (e) {
        return fail(500, `agents unavailable: ${e}`);
    }
    if (rows === null || Array.isArray(rows) === false) {
        return fail(500, 'agents unavailable');
    }
    const items = rows.map((u) => ({
        id: String(u.id),
        name: u.display_name ?? u.username ?? String(u.id),
        email: null,
        avatar_url: u.avatar ?? null,
        availability: 'online',
    }));
    return { items };
}

// Presence heartbeat moved to the kernel primitive (`POST /api/v1/presence/heartbeat`),
// which owns the presence store and emits presence.* events (architecture §5.3).
// The workspace frontend calls the kernel endpoint directly — no plugin route needed.

// ── chat.egress ───────────────────────────────────────────────
// Outbound dispatch for an agent/assistant message. Reads chat_inbox.egress
// ({kind: sse|api, client, op, ...}); v1: kind=sse → mark delivered (SSE was
// already pushed), kind=api → callApi passthrough. IM/email templates land in
// CH-3.

export function onEgress(input) {
    const job = parseJobInput(input);
    const { message_id } = job;
    if (!message_id) throw new Error('chat.egress: missing message_id');

    const msg = ctGet(CT_MSG, message_id);
    if (!msg) throw new Error(`chat.egress: message ${message_id} not found`);
    if (!msg.conversation_id) {
        ctUpdate(CT_MSG, message_id, { status: 'delivered' });
        return { status: 'delivered' };
    }

    const conv = ctGet(CT_CONV, String(msg.conversation_id));
    if (!conv) throw new Error(`chat.egress: conversation ${msg.conversation_id} not found`);

    const inbox = conv.inbox_id ? ctGet(CT_INBOX, String(conv.inbox_id)) : null;
    const egress = inbox?.egress;
    const kind = egress?.kind ?? 'sse';

    if (kind === 'sse' || !egress?.client) {
        ctUpdate(CT_MSG, message_id, { status: 'delivered' });
        return { status: 'delivered', kind: 'sse' };
    }

    // kind=api → callApi passthrough (feishu/telegram/whatsapp in CH-3).
    try {
        const payload = {
            message: msg,
            conversation: conv,
            channel_key: inbox?.channel_id != null ? String(inbox.channel_id) : null,
        };
        if (egress.input) payload.input = egress.input;
        callApi(egress.client, egress.op ?? 'send', payload);
        ctUpdate(CT_MSG, message_id, { status: 'delivered' });
        return { status: 'delivered', kind: 'api' };
    } catch (e) {
        ctUpdate(CT_MSG, message_id, { status: 'failed' });
        eventEmit('chat.alert', {
            conversation_id: idOf(conv),
            message_id: message_id,
            reason: String(e),
        });
        throw e;
    }
}

// ── Widget routes (CH-1, W3/W4) ───────────────────────────────
// Visitor-facing endpoints (widget.md §5). `auth = public` on the manifest —
// handlers validate the short-session widget JWT (Bearer) via host.verifyToken.
// Widget tokens are scoped to a contact+channel, so cross-session access is
// impossible (server-side enforced here and in /events/session).

function widgetAuth(input) {
    const authHeader = input?.headers?.authorization ?? input?.headers?.Authorization;
    if (!authHeader || !String(authHeader).toLowerCase().startsWith('bearer ')) return null;
    const token = String(authHeader).slice(7).trim();
    if (!token) return null;
    return verifyToken(token);
}

// POST /api/v1/plugins/chat/widget/session
export function widgetSession(input) {
    const data = routeInput(input);
    const body = jsonBody(data);
    const channelKey = body.channel_key;
    if (!channelKey) return invalid('channel_key required');

    // Anonymous v1 identity: the frontend's persistent visitor id doubles as
    // the sender (localStorage). HMAC identity (identifier+signature) is a
    // documented later enhancement (widget.md §2).
    const visitorId = typeof body.visitor_id === 'string' && body.visitor_id
        ? body.visitor_id
        : newId();

    // Resolve the contact by the persistent visitor id, then wire a
    // (channel, contact_id) identity so the ingress path — which attributes
    // the sender from the token claims (contact_id) — merges into the SAME
    // conversation (widget.md §2/§3.1).
    // NOTE: contact ids are the PLAIN digit form everywhere on the wire
    // (idOf() returns base62-encoded PK ids; the identity/conversation rows
    // read `contact_id` back as digits). Keep `contactId` in digit form so
    // token sub == identity sender == conversation.contact_id all agree.
    let contactId = null;
    const byVisitor = findFirst(CT_IDENTITY, [
        { field: 'channel', value: channelKey },
        { field: 'sender', value: visitorId },
    ]);
    if (byVisitor) {
        contactId = String(byVisitor.contact_id);
    } else {
        const contact = ctCreate(CT_CONTACT, { name: visitorId });
        const ident = ctCreate(CT_IDENTITY, {
            contact_id: idOf(contact),
            channel: channelKey,
            sender: visitorId,
        });
        contactId = ident.contact_id != null ? String(ident.contact_id) : idOf(contact);
    }
    const byContact = findFirst(CT_IDENTITY, [
        { field: 'channel', value: channelKey },
        { field: 'sender', value: contactId },
    ]);
    if (!byContact) {
        ctCreate(CT_IDENTITY, {
            contact_id: contactId,
            channel: channelKey,
            sender: contactId,
        });
    }

    // Map channel_key → itg_channel.id → chat_inbox (for bot/greeting).
    // CAST to TEXT: dbQuery returns bigint ids as JS Numbers and snowflakes
    // exceed Number.MAX_SAFE_INTEGER (precision loss).
    let inbox = null;
    try {
        const rows = dbQuery(
            `SELECT CAST(id AS TEXT) AS id FROM itg_channels WHERE channel_key = ${dbPh(1)}`,
            [channelKey],
        );
        const channelId = rows && rows[0] && rows[0].id ? String(rows[0].id) : null;
        if (channelId) {
            inbox = findFirst(CT_INBOX, [{ field: 'channel_id', value: channelId }]);
        }
    } catch (e) {
        logWarn(`[chat] widget/session channel lookup failed: ${e}`);
    }

    const botBound = !!(inbox && inbox.bot_id != null && inbox.bot_id !== '');
    const conv = ensureConversation(contactId, inbox, botBound);

    const issued = issueToken({ channel_key: channelKey, contact_id: contactId, ttl_secs: 7200 });
    return {
        token: issued.token,
        contact_id: contactId,
        conversation_id: conv.id,
        inbox_id: idOf(inbox),
        greeting: inbox?.greeting ?? null,
    };
}

// GET /api/v1/plugins/chat/widget/messages?conversation=&since=
export function widgetMessages(input) {
    const data = routeInput(input);
    const session = widgetAuth(data);
    if (!session) return fail(401, 'invalid widget token');

    const conversation = data.query?.conversation;
    if (!conversation) return invalid('conversation required');
    const conv = ctGet(CT_CONV, String(conversation));
    if (!conv) return notFound(`conversation ${conversation}`);
    if (String(conv.contact_id) !== session.contact_id) return fail(403, 'forbidden');

    const filters = [{ field: 'conversation_id', value: String(conversation) }];
    if (data.query?.since) filters.push({ field: 'id', op: 'gt', value: String(data.query.since) });
    const res = ctFind(CT_MSG, { filters, sort: 'id asc', page_size: 100 });
    return { items: res.rows ?? [] };
}

// POST /api/v1/plugins/chat/widget/typing
export function widgetTyping(input) {
    const data = routeInput(input);
    const session = widgetAuth(data);
    if (!session) return fail(401, 'invalid widget token');
    const conversation = pathParam(data, 'conversation') ?? data.query?.conversation;
    if (!conversation) return invalid('conversation required');
    eventEmit('chat.typing', { conversation_id: String(conversation), side: 'visitor' });
    return { ok: true };
}

// POST /api/v1/plugins/chat/widget/read
export function widgetRead(input) {
    const data = routeInput(input);
    const session = widgetAuth(data);
    if (!session) return fail(401, 'invalid widget token');
    const conversation = pathParam(data, 'conversation') ?? data.query?.conversation;
    if (!conversation) return invalid('conversation required');
    const conv = ctGet(CT_CONV, String(conversation));
    if (!conv || String(conv.contact_id) !== session.contact_id) return fail(403, 'forbidden');
    ctUpdate(CT_CONV, String(conversation), { visitor_last_seen_at: nowISO() });
    return { ok: true };
}
