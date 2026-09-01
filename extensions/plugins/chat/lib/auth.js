// Authentication & authorization helpers for workspace routes.
//
// Visibility policy: platform admins see all; agents see own + unassigned.
// (Team visibility lands in CH-2 with chat_team.) Filtering happens in JS
// because CT where-DSL has no OR across assignee/None.

export function isAdmin(auth) {
    return Array.isArray(auth?.roles) && auth.roles.includes('admin');
}

export function agentId(input) {
    return input?.auth?.user_id ? String(input.auth.user_id) : null;
}

export function visibleRows(rows, auth) {
    if (isAdmin(auth)) return rows;
    const me = String(auth?.user_id ?? '');
    if (!me) return rows.filter((r) => r.assignee_id == null);
    return rows.filter((r) => r.assignee_id == null || String(r.assignee_id) === me);
}

// Widget tokens are short-session JWTs issued by host.issueToken and
// validated via host.verifyToken (widget.md §2/§3.2). Cross-session access
// is impossible: handlers must additionally scope reads to session.contact_id.
import { verifyToken } from 'sdk';

export function widgetAuth(input) {
    const authHeader = input?.headers?.authorization ?? input?.headers?.Authorization;
    if (!authHeader || !String(authHeader).toLowerCase().startsWith('bearer ')) return null;
    const token = String(authHeader).slice(7).trim();
    if (!token) return null;
    return verifyToken(token);
}
