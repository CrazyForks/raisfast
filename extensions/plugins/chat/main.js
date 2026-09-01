// Chat application plugin — thin entry that re-exports every job handler and
// route handler declared in manifest.toml. Business logic lives in the
// services/ and routes/ modules (and shared helpers in lib/); this file only
// aggregates the exports so the manifest's `handler = "..."` references stay
// stable as the codebase grows (CH-2+ assign/sla/csat/automation).
//
// Jobs:
//   chat.ingress   (enqueued by the integration pipeline route_extra.jobs)
//   chat.autoreply (enqueued by chat.ingress; fallback mode uses delay)
//   chat.egress    (enqueued by sendMessage)
//
// Routes: workspace + widget (see routes/). The kernel owns presence
// (architecture §5.3) — no heartbeat route here.

// ── Jobs ──────────────────────────────────────────────────────
export { onIngress } from './services/ingress.js';
export { onAutoreply } from './services/autoreply.js';
export { onEgress } from './services/egress.js';
export { onAssign, onAssignScan } from './services/assign.js';

// ── Widget routes (CH-1, W3/W4) ───────────────────────────────
export { widgetSession, widgetMessages, widgetTyping, widgetRead } from './routes/widget.js';

// ── Workspace routes (CH-1) ───────────────────────────────────
export {
    listConversations,
    getConversation,
    listMessages,
    sendMessage,
    assignConversation,
    unassignConversation,
    assignTeam,
    updateStatus,
    toggleBot,
    updateLabels,
    markConversationRead,
} from './routes/conversations.js';
export { listContacts, getContactTimeline, listAgents } from './routes/contacts.js';
