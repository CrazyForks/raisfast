// Inbox channel wizard (channel-app-ownership.md §5.2) — app-scoped channel
// management through the integration host API (`integration = ["channels"]`)
// plus the chat_inbox CT binding. v1 ships the widget template; CH-2/CH-3 add
// feishu/dingtalk/telegram/email templates. `app_id` is derived from the
// plugin id by the host — this file never sends one.

import {
    channelList,
    channelCreate,
    channelUpdate,
    channelDelete,
    ctCreate,
    ctFind,
    ctUpdate,
    decodeId,
    newId,
} from 'sdk';
import {
    CT_INBOX,
    jsonBody,
    pathParam,
    routeInput,
    idOf,
} from '../lib/ctx.js';
import { invalid, fail } from '../lib/errors.js';

// v1 channel template: the web widget (mirrors apps/chat/channels/widget.json,
// minus channel_key which the wizard mints, and credentials which never ship).
const WIDGET_TEMPLATE = {
    provider: 'widget',
    display_name: '网页 Widget',
    mode: 'push',
    transport: 'http1',
    framing: 'raw',
    codec: 'json',
    verify_kind: 'jwt-widget',
    mapping: {
        external_id: '$.id',
        sender: '$.sender',
        payload: { body: '$.text' },
    },
    target_type: 'chat/chat_messages',
    route_extra: {
        jobs: [{ job_type: 'chat.ingress', max_attempts: 1 }],
    },
};

// Channel rows cross the plugin boundary with a base62-encoded id
// (ID_ENCODING); chat_inbox.channel_id is a plain digit string. Normalize the
// channel id so the workspace can join the two by the same snowflake id.
function toWorkspaceChannel(ch) {
    return {
        ...ch,
        id: ch.id != null ? decodeId(String(ch.id)) : ch.id,
    };
}

// GET /api/v1/plugins/chat/admin/channels
export function listInboxChannels(input) {
    let channels;
    try {
        channels = channelList() ?? [];
    } catch (e) {
        return fail(500, `channels unavailable: ${e}`);
    }
    if (!Array.isArray(channels)) return fail(500, 'channels unavailable');

    const inboxRows = ctFind(CT_INBOX, { page_size: 500 }).rows ?? [];
    const byChannel = new Map(inboxRows.map((r) => [String(r.channel_id), r]));
    const items = channels.map((ch) => {
        const norm = toWorkspaceChannel(ch);
        const inbox = byChannel.get(norm.id) ?? null;
        // Decode the inbox's PK id to a plain digit string too — conversation
        // rows expose inbox_id as digits while CT `id` is base62 (ID_ENCODING),
        // so the sidebar can join them by the same id.
        return {
            ...norm,
            inbox: inbox
                ? { ...inbox, id: decodeId(String(inbox.id)), channel_id: norm.id }
                : null,
        };
    });
    return { items };
}

// POST /api/v1/plugins/chat/admin/channels
// body: { channel?: {...}, inbox: { name, greeting?, auto_assignment? } }
// Atomic: creates the app-owned channel, then the chat_inbox row bound to it.
// On inbox failure the channel is rolled back so the wizard leaves no orphans.
export function createInboxChannel(input) {
    const data = routeInput(input);
    const body = jsonBody(data);
    const inbox = body.inbox ?? {};
    const name = String(inbox.name ?? '').trim();
    if (!name) return invalid('inbox.name is required');

    const channelInput = { ...WIDGET_TEMPLATE, ...(body.channel ?? {}) };
    if (!channelInput.channel_key) {
        channelInput.channel_key = `chat-widget-${String(newId()).slice(-10)}`;
    }

    let created;
    try {
        created = channelCreate(JSON.stringify(channelInput));
    } catch (e) {
        return fail(500, `channel create failed: ${e}`);
    }
    if (!created || created.error) return fail(500, created?.error ?? 'channel create failed');

    const inboxRow = {
        channel_id: String(created.id),
        name,
        greeting: inbox.greeting ? String(inbox.greeting) : null,
        auto_assignment: Boolean(inbox.auto_assignment),
    };
    let createdInbox;
    try {
        createdInbox = ctCreate(CT_INBOX, inboxRow);
    } catch (e) {
        try {
            channelDelete(String(created.id));
        } catch {
            /* best-effort rollback */
        }
        return fail(500, `inbox create failed: ${e}`);
    }

    return {
        channel: toWorkspaceChannel(created),
        inbox: { ...createdInbox, channel_id: decodeId(String(created.id)) },
    };
}

// PUT /api/v1/plugins/chat/admin/channels/:id
// body: { channel?: { display_name?, enabled?, ... }, inbox?: { name?, greeting?, auto_assignment? } }
export function updateInboxChannel(input) {
    const data = routeInput(input);
    const id = pathParam(data, 'id');
    if (!id) return invalid('missing channel id');
    const body = jsonBody(data);

    let ch;
    try {
        ch = channelUpdate(String(id), JSON.stringify(body.channel ?? {}));
    } catch (e) {
        return fail(500, `channel update failed: ${e}`);
    }
    if (!ch || ch.error) return fail(500, ch?.error ?? 'channel update failed');

    let updated = null;
    const normId = decodeId(String(ch.id));
    const inboxRows =
        ctFind(CT_INBOX, {
            filters: [{ field: 'channel_id', value: normId }],
            page_size: 1,
        }).rows ?? [];
    if (inboxRows[0]) {
        const patch = {};
        if (body.inbox?.name != null) patch.name = String(body.inbox.name);
        if (body.inbox?.greeting != null) {
            patch.greeting = body.inbox.greeting ? String(body.inbox.greeting) : null;
        }
        if (body.inbox?.auto_assignment != null) {
            patch.auto_assignment = Boolean(body.inbox.auto_assignment);
        }
        if (Object.keys(patch).length > 0) {
            updated = ctUpdate(CT_INBOX, idOf(inboxRows[0]), patch);
        } else {
            updated = inboxRows[0];
        }
    }
    return { channel: toWorkspaceChannel(ch), inbox: updated };
}
