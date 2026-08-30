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
    jobEnqueue, getReceipt, callApi, eventEmit, logInfo, logWarn,
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
        message_id: idOf(assistant),
        role: 'assistant',
        body: replyText,
    });
    eventEmit('chat.message.created', {
        trace_id: traceId,
        channel: channelKey,
        conversation_id: convId,
        message_id: idOf(assistant),
        role: 'assistant',
        body: replyText,
    });
    logInfo(`[chat] autoreply delivered trace=${traceId} conv=${convId}`);
    return { conversation_id: convId };
}
