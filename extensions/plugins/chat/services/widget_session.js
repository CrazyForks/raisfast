// Widget session bootstrap — the visitor identity + conversation wiring
// (widget.md §4.1 / §2). Pure logic over the CT host APIs: anonymous visitor
// id → contact/identity merge → conversation locate → session token.
// Thin route handlers live in routes/widget.js; this holds the business logic
// so it is unit-testable without the HTTP layer.

import {
    ctCreate,
    ctFind,
    ctGet,
    dbPh,
    dbQuery,
    issueToken,
    logWarn,
    newId,
} from 'sdk';
import {
    CT_CONTACT,
    CT_IDENTITY,
    CT_INBOX,
    findFirst,
    idOf,
    routeInput,
    jsonBody,
} from '../lib/ctx.js';
import { invalid } from '../lib/errors.js';
import { ensureConversation } from './inbox.js';

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

    const contactId = resolveContactId(channelKey, visitorId);
    const inbox = resolveInbox(channelKey);
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

// Resolve the contact by the persistent visitor id, then wire a
// (channel, contact_id) identity so the ingress path — which attributes
// the sender from the token claims (contact_id) — merges into the SAME
// conversation (widget.md §2/§3.1).
// NOTE: contact ids are the PLAIN digit form everywhere on the wire
// (idOf() returns base62-encoded PK ids; the identity/conversation rows
// read `contact_id` back as digits). Keep `contactId` in digit form so
// token sub == identity sender == conversation.contact_id all agree.
export function resolveContactId(channelKey, visitorId) {
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
    return contactId;
}

// Map channel_key → itg_channel.id → chat_inbox (for bot/greeting).
// CAST to TEXT: dbQuery returns bigint ids as JS Numbers and snowflakes
// exceed Number.MAX_SAFE_INTEGER (precision loss).
export function resolveInbox(channelKey) {
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
    return inbox;
}
