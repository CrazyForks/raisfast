// `{var}` template renderer shared by chat.egress (outbound API payloads) and
// chat.ingress (contact enrichment). Renders dotted paths from a context
// object: `{reply.chat_id}`, `{msg.body}`, `{conv.contact_id}`, `{sender}`.
// Missing values render as empty strings; scalars are stringified.

function resolvePath(ctx, path) {
  let v = ctx;
  for (const part of String(path).split('.')) {
    if (v == null) return null;
    v = v[part];
  }
  return v == null ? null : String(v);
}

export function renderTemplate(tpl, ctx) {
  if (typeof tpl === 'string') {
    return tpl.replace(/\{([\w.]+)\}/g, (_, key) => resolvePath(ctx, key) ?? '');
  }
  if (Array.isArray(tpl)) return tpl.map((x) => renderTemplate(x, ctx));
  if (tpl && typeof tpl === 'object') {
    const out = {};
    for (const [k, v] of Object.entries(tpl)) out[k] = renderTemplate(v, ctx);
    return out;
  }
  return tpl;
}
