// chat.egress — outbound dispatch for an agent/assistant message
// (architecture §4.2). Dispatch by `chat_inbox.egress`:
//   kind=sse      → mark delivered (widget; SSE was already pushed live)
//   kind=api      → callApi passthrough to a declarative api-client
//   kind=webhook  → httpPost to the conversation's per-channel reply target
// The channel shape is driven by conv.reply_to (captured by chat.ingress):
//   feishu   → callApi(egress.client, 'send_text', {receive_id: chat_id, ...})
//   dingtalk → httpPost(reply_to.webhook, {msgtype:'text', text:{content}})

import {
    callApi,
    ctGet,
    ctUpdate,
    httpPost,
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
    const reply = conv.reply_to ?? null;

    // No reply capability → SSE channel (widget): SSE already pushed live.
    if (kind === 'sse' || (!egress?.client && !reply?.webhook)) {
        ctUpdate(CT_MSG, message_id, { status: 'delivered' });
        return { status: 'delivered', kind: 'sse' };
    }

    try {
        if (kind === 'webhook' && reply?.webhook) {
            // DingTalk bot reply: per-message sessionWebhook (self-auth).
            const resp = httpPost(
                String(reply.webhook),
                JSON.stringify({ msgtype: 'text', text: { content: msg.body ?? '' } }),
            );
            if (!httpPostOk(resp)) throw new Error(String(resp).slice(0, 300));
        } else if (kind === 'api' && egress?.client) {
            callApi(egress.client, egress.op ?? 'send', buildApiInput(reply, egress, msg, conv));
        } else {
            throw new Error(`chat.egress: no dispatch for kind=${kind} reply=${JSON.stringify(reply)}`);
        }
        ctUpdate(CT_MSG, message_id, { status: 'delivered' });
        return { status: 'delivered', kind };
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

// Shape the callApi input for an IM reply. feishu `send_text` op expects
// {receive_id, msg_type, content}; other clients get the generic passthrough
// (message/conversation/reply) unless egress.input overrides the shape.
function buildApiInput(reply, egress, msg, conv) {
    const channel = reply?.channel;
    if (channel === 'feishu') {
        return {
            receive_id: reply?.chat_id ?? reply?.open_id ?? '',
            receive_id_type: reply?.chat_id ? 'chat_id' : 'open_id',
            msg_type: 'text',
            content: JSON.stringify({ text: msg.body ?? '' }),
        };
    }
    const payload = {
        message: msg,
        conversation: conv,
        reply,
    };
    if (egress?.input) payload.input = egress.input;
    return payload;
}

// The host `httpPost` returns `{"status":<code>,"body":...}` on an HTTP reply,
// or an `error:`-prefixed string on network/whitelist failure. Fail closed on
// any non-2xx so the message is marked failed + alerted instead of silently
// "delivered".
function httpPostOk(resp) {
    const s = String(resp);
    if (s.startsWith('error:')) return false;
    const m = s.match(/"status":\s*(\d{3})/);
    if (m) return Number(m[1]) >= 200 && Number(m[1]) < 300;
    return true;
}
