// Widget visitor routes (widget.md §5) — thin HTTP handlers. The session
// bootstrap business logic lives in services/widget_session.js; messages/
// typing/read here only do token auth + ownership checks + host calls.
// `auth = public` on the manifest — handlers validate the short-session
// widget JWT (Bearer) via host.verifyToken (lib/auth.widgetAuth). Widget
// tokens are scoped to a contact+channel, so cross-session access is
// impossible (server-side enforced here and in /events/session).

import {
    ctFind,
    ctGet,
    ctUpdate,
} from 'sdk';
import {
    CT_CONV,
    CT_MSG,
    idOf,
    nowISO,
    pathParam,
    routeInput,
} from '../lib/ctx.js';
import { widgetAuth } from '../lib/auth.js';
import { invalid, notFound, fail } from '../lib/errors.js';
import { emitTyping } from '../lib/events.js';

export { widgetSession } from '../services/widget_session.js';

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
    emitTyping({ conversation_id: String(conversation), side: 'visitor' });
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
