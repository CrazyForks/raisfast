// Workspace contact + agent routes. Contact timeline reuses the conversation
// visibility policy; the agents list reads platform users via dbQuery (users
// is not a CT). Live presence rides SSE presence.* events (architecture §5.3).

import {
    ctFind,
    ctGet,
    dbQuery,
} from 'sdk';
import {
    CT_CONTACT,
    CT_CONV,
    CT_IDENTITY,
    idOf,
    pathParam,
    routeInput,
} from '../lib/ctx.js';
import { visibleRows } from '../lib/auth.js';
import { fail, invalid, notFound } from '../lib/errors.js';

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
    // Denormalize contact name/email onto the conversation rows.
    const decorate = (row) => ({
        ...row,
        contact_name: contact?.name ?? null,
        contact_email: contact?.email ?? null,
        contact_avatar_url: contact?.avatar_url ?? null,
    });

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
