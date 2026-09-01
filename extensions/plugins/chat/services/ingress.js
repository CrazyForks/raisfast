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
    enrichContact(channelKey, sender, contactId);
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
    if (!chatId && !openId && !webhook) return null;
    const reply = { channel: channelKey };
    if (chatId) reply.chat_id = String(chatId);
    if (openId) reply.open_id = String(openId);
    if (webhook) reply.webhook = String(webhook);
    return reply;
}

// Enrich a contact's display name from the IM provider once, so the workspace
// shows a real name instead of the raw open_id/staffId. Runs only when the
// contact is still named after the sender (not yet resolved); best-effort —
// any provider error keeps the sender id as the name.
export function enrichContact(channelKey, sender, contactId) {
    const contact = ctGet(CT_CONTACT, contactId);
    if (!contact) return;
    const name = contact.name;
    if (name && name !== sender) return; // already resolved

    try {
        if (channelKey === 'feishu') {
            const outRaw = callApi('feishu', 'get_user', { user_id: String(sender) });
            const out = typeof outRaw === 'string' ? JSON.parse(outRaw) : outRaw;
            if (out && out.error) throw new Error(out.error);
            const v = out?.output ?? out;
            const realName = v?.name ?? v?.data?.name;
            if (realName) {
                const patch = { name: String(realName) };
                if (v.avatar_url) patch.avatar_url = String(v.avatar_url);
                ctUpdate(CT_CONTACT, contactId, patch);
                logInfo(`[chat] feishu contact enriched: ${sender} → ${realName}`);
            }
        }
        // dingtalk/telegram/... per-channel resolvers land in CH-3.
    } catch (e) {
        logWarn(`[chat] contact enrich failed (${channelKey}/${sender}): ${e}`);
    }
}
