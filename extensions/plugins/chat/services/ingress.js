// chat.ingress — the post-route merge for every inbound message
// (architecture §4.1). Enqueued by the integration pipeline's
// route_extra.jobs with { trace_id, channel_key }.
//
// Flow: identity merge → conversation locate/create → message back-link →
// denormalized counters → SSE → bot gate (inbox.bot_id ∧ bot.enabled ∧
// conversation.bot_status=active → enqueue chat.autoreply).

import {
    callApi,
    ctGet,
    ctUpdate,
    getReceipt,
    jobEnqueue,
    logInfo,
    logWarn,
} from 'sdk';
import {
    CT_BOT,
    CT_CONTACT,
    CT_CONV,
    CT_INBOX,
    findFirst,
    idOf,
    parseJobInput,
} from '../lib/ctx.js';
import { emitMessageCreated } from '../lib/events.js';
import { renderTemplate } from '../lib/template.js';
import {
    ensureConversation,
    ensureIdentity,
    linkMessage,
    touchConversation,
} from './inbox.js';

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
    enrichContact(channelKey, sender, contactId, inbox);
    const conv = ensureConversation(contactId, inbox, bot != null);
    const messageId = linkMessage(traceId, externalId, conv.id, conv.isNew);
    touchConversation(conv.id, conv.row);

    // Persist the outbound reply target (per channel, from the mapping):
    //   feishu  → reply_chat_id  (event.message.chat_id)
    //   dingtalk → reply_webhook (data.sessionWebhook, per-message URL)
    // chat.egress reads conv.reply_to to send agent replies back.
    const replyTo = buildReplyTo(channelKey, env);
    if (replyTo) {
        ctUpdate(CT_CONV, conv.id, { reply_to: replyTo });
    }

    emitMessageCreated({
        trace_id: traceId,
        channel: channelKey,
        conversation_id: conv.id,
        contact_id: contactId,
        message_id: messageId,
        role: 'user',
        body,
        last_message_at: new Date().toISOString(),
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

// Derive the per-channel outbound reply target from the mapped envelope
// payload (chat_message.reply_chat_id / reply_webhook). Null when the channel
// has no reply capability (e.g. widget SSE — egress falls back to sse).
export function buildReplyTo(channelKey, env) {
    const payload = env?.payload ?? {};
    const chatId = payload.reply_chat_id ?? payload.chat_id ?? null;
    const openId = payload.reply_open_id ?? payload.open_id ?? null;
    const webhook = payload.reply_webhook ?? payload.webhook ?? null;
    const groupId = payload.reply_group_id ?? payload.group_id ?? null;
    const messageType = payload.reply_message_type ?? payload.message_type ?? null;
    if (!chatId && !openId && !webhook && !groupId) return null;
    const reply = { channel: channelKey };
    if (chatId) reply.chat_id = String(chatId);
    if (openId) reply.open_id = String(openId);
    if (webhook) reply.webhook = String(webhook);
    if (groupId) reply.group_id = String(groupId);
    if (messageType) reply.message_type = String(messageType);
    return reply;
}

// Enrich a contact's display name from the IM provider once, so the workspace
// shows a real name instead of the raw open_id/staffId. Driven by the inbox's
// `enrich` config (architecture §4.1 3c): {client, op, input template,
// name mapping, avatar path}. Runs only when the contact is still named after
// the sender; best-effort — any provider error keeps the sender id as the name.
export function enrichContact(channelKey, sender, contactId, inbox) {
    const contact = ctGet(CT_CONTACT, contactId);
    if (!contact) return;
    const name = contact.name;
    if (name && name !== sender) return; // already resolved

    const enrich = inbox?.enrich;
    if (!enrich?.client) return; // channel has no enrichment declared
    try {
        const ctx = { sender: String(sender), chat_id: String(sender) };
        const outRaw = callApi(
            enrich.client,
            enrich.op ?? 'get_profile',
            renderTemplate(enrich.input ?? {}, ctx),
        );
        const out = typeof outRaw === 'string' ? JSON.parse(outRaw) : outRaw;
        if (out && out.error) throw new Error(out.error);
        const v = out?.output ?? out;

        const patch = {};
        const realName = buildEnrichedName(v, enrich.name);
        if (realName) patch.name = realName;
        if (enrich.avatar) {
            const av = lookupPath(v, enrich.avatar);
            if (av) patch.avatar_url = String(av);
        }
        if (Object.keys(patch).length > 0) {
            ctUpdate(CT_CONTACT, contactId, patch);
            logInfo(`[chat] contact enriched: ${sender} → ${realName ?? '?'}`);
        }
    } catch (e) {
        logWarn(`[chat] contact enrich failed (${channelKey}/${sender}): ${e}`);
    }
}

// Resolve a display name from the provider output per the `name` config:
//   string                → dot path (e.g. "name")
//   ["first_name","last"] → join the found parts with a space
//   {join:[...], sep?, fallback?} → join parts, else fallback path
function buildEnrichedName(v, nameCfg) {
    if (!nameCfg) return null;
    if (typeof nameCfg === 'string') return lookupPath(v, nameCfg);
    if (Array.isArray(nameCfg)) {
        const parts = nameCfg.map((p) => lookupPath(v, p)).filter(Boolean);
        return parts.length ? parts.join(' ') : null;
    }
    if (typeof nameCfg === 'object') {
        const join = Array.isArray(nameCfg.join) ? nameCfg.join : [];
        const parts = join.map((p) => lookupPath(v, p)).filter(Boolean);
        if (parts.length) return parts.join(nameCfg.sep ?? ' ');
        if (nameCfg.fallback) return lookupPath(v, nameCfg.fallback);
    }
    return null;
}

function lookupPath(v, path) {
    let cur = v;
    for (const p of String(path).split('.')) {
        if (cur == null) return null;
        cur = cur[p];
    }
    return cur == null ? null : String(cur);
}
