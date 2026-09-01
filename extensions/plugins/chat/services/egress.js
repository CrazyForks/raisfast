// chat.egress — outbound dispatch for an agent/assistant message
// (architecture §4.2). Reads chat_inbox.egress ({kind: sse|api, client,
// op, ...}); v1: kind=sse → mark delivered (SSE was already pushed),
// kind=api → callApi passthrough. IM/email templates land in CH-3.

import {
    callApi,
    ctGet,
    ctUpdate,
} from 'sdk';
import {
    CT_CONV,
    CT_INBOX,
    CT_MSG,
    idOf,
    parseJobInput,
} from '../lib/ctx.js';
import { emitAlert } from '../lib/events.js';

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
        emitAlert({
            conversation_id: idOf(conv),
            message_id: message_id,
            reason: String(e),
        });
        throw e;
    }
}
