// Conversation/contact lifecycle primitives shared by the ingress job and
// the widget bootstrap. Pure functions over the CT host APIs — the bot
// binding decision and reopen semantics live here so both callers stay in
// lock-step (architecture §4.1 steps 1–3).

import {
    ctCreate,
    ctUpdate,
} from 'sdk';
import {
    CT_CONTACT,
    CT_CONV,
    CT_IDENTITY,
    CT_MSG,
    findFirst,
    idOf,
    nowISO,
} from '../lib/ctx.js';

// Identity merge: one (channel, sender) → one contact. Creates the contact
// on first sight, then the identity row links the sender to it.
export function ensureIdentity(channelKey, sender) {
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

// Conversation locate/reuse: open|pending session wins; else reopen the
// latest resolved conversation (when reopen_enabled); else create a new one.
// `botBound` decides the initial state (pending+active for bot, open+disabled
// for human-only).
export function ensureConversation(contactId, inbox, botBound) {
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

// Back-link a routed raw message row to its conversation. The pipeline route
// drops the message with a receipt_id injected; this attaches conversation_id.
export function linkMessage(traceId, externalId, conversationId, isNew) {
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

// Denormalized conversation touch: last message + unread counter (avoids
// COUNT(*) on the hot path; architecture §3.1).
export function touchConversation(convId, convRow) {
    ctUpdate(CT_CONV, convId, {
        last_message_at: nowISO(),
        last_message_role: 'user',
        unread_count: (convRow?.unread_count ?? 0) + 1,
    });
}
