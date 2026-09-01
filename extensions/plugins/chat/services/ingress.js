// chat.ingress — the post-route merge for every inbound message
// (architecture §4.1). Enqueued by the integration pipeline's
// route_extra.jobs with { trace_id, channel_key }.
//
// Flow: identity merge → conversation locate/create → message back-link →
// denormalized counters → SSE → bot gate (inbox.bot_id ∧ bot.enabled ∧
// conversation.bot_status=active → enqueue chat.autoreply).

import {
    ctGet,
    getReceipt,
    jobEnqueue,
    logInfo,
} from 'sdk';
import {
    CT_BOT,
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
    const conv = ensureConversation(contactId, inbox, bot != null);
    const messageId = linkMessage(traceId, externalId, conv.id, conv.isNew);
    touchConversation(conv.id, conv.row);

    emitMessageCreated({
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
