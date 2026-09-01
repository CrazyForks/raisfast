// In-memory mock of the plugin `sdk` module for unit tests.
// Services import named host functions from `sdk`; the loader maps that
// specifier to this module so inbox/autoreply/widget_session logic can run
// in Node without the kernel. Provides test controls (`__reset`, `__seed`,
// `__rows`, `__emitted`) plus functional ct.*/dbQuery/event/LLM stubs.

// ── state ─────────────────────────────────────────────────────

const tables = new Map();       // normalized name -> rows[]
const emitted = [];
const receipts = new Map();     // trace_id -> { channel_id, envelope }
const channels = new Map();     // channel_key -> { id, ... }
let idSeq = 1;

function norm(ct) {
  // "chat/chat_messages" → "chat_messages"; also accept bare names.
  return String(ct).split("/").pop();
}

function table(name) {
  const n = norm(name);
  if (!tables.has(n)) tables.set(n, []);
  return tables.get(n);
}

// ── filter matching (mirrors the CT where-DSL subset the plugin uses) ──

function matches(row, filter) {
  const val = row?.[filter.field];
  switch (filter.op ?? "eq") {
    case "eq": return String(val ?? "") === String(filter.value);
    case "ne": return String(val ?? "") !== String(filter.value);
    case "gt": return String(val ?? "") > String(filter.value);
    case "gte": return String(val ?? "") >= String(filter.value);
    case "lt": return String(val ?? "") < String(filter.value);
    case "lte": return String(val ?? "") <= String(filter.value);
    case "in": return Array.isArray(filter.value)
      ? filter.value.some((v) => String(val ?? "") === String(v))
      : false;
    case "contains": return String(val ?? "").includes(String(filter.value));
    case "like": return String(val ?? "").includes(String(filter.value));
    default: return false;
  }
}

// ── host API stubs (named exports, same as the real sdk) ─────

export function ctFind(ct, query) {
  const { filters = [], sort = "id desc", page_size = 50 } = query ?? {};
  let rows = table(ct).filter((r) => filters.every((f) => matches(r, f)));
  const [field, dir] = String(sort).split(" ");
  rows = rows.sort((a, b) => {
    const av = String(a[field] ?? "");
    const bv = String(b[field] ?? "");
    return dir === "asc" ? (av < bv ? -1 : av > bv ? 1 : 0) : av > bv ? -1 : av < bv ? 1 : 0;
  });
  return { rows: rows.slice(0, page_size) };
}

export function ctGet(ct, id) {
  return table(ct).find((r) => String(r.id) === String(id)) ?? null;
}

export function ctCreate(ct, data) {
  const row = { ...data, id: String(idSeq++) };
  table(ct).push(row);
  return row;
}

export function ctUpdate(ct, id, patch) {
  const row = ctGet(ct, id);
  if (row) Object.assign(row, patch);
  return row;
}

export function jobEnqueue() { /* recorded for assertion */ emitted.push({ kind: "job", args: [...arguments] }); }

export function getReceipt(traceId) {
  return receipts.get(String(traceId)) ?? null;
}

export function callApi(client, op, input) {
  emitted.push({ kind: "callApi", client, op, input });
  return JSON.stringify({ status: "ok", output: __llmReply ?? "mock reply" });
}

export function eventEmit(type, data) {
  emitted.push({ kind: "event", type, data });
}

export function logInfo() {}
export function logWarn() {}

export function dbQuery(sql, params) {
  const key = String(sql.match(/channel_key = (\w+)/)?.[1] ?? "");
  const row = channels.get(key);
  return row ? [{ id: String(row.id) }] : [];
}

export function dbPh() { return "?"; }
export function newId() { return `mock-visitor-${idSeq++}`; }
export function issueToken(input) {
  const o = JSON.parse(String(input));
  emitted.push({ kind: "issueToken", ...o });
  return { token: `mock-token.${o.channel_key}.${o.contact_id}` };
}
export function verifyToken() { return null; }

// Presence host API mock: returns the seeded available set (digit strings).
const presenceAvailableSet = [];
export function __seedPresenceAvailable(ids) {
  presenceAvailableSet.length = 0;
  presenceAvailableSet.push(...ids.map(String));
}
export function presenceAvailable() { return [...presenceAvailableSet]; }
export function presenceStatus() { return "online"; }
export function presenceReport() { return { ok: true }; }

// ── test controls ─────────────────────────────────────────────

export function __reset() {
  tables.clear();
  emitted.length = 0;
  receipts.clear();
  channels.clear();
  presenceAvailableSet.length = 0;
  idSeq = 1;
}

export function __seed(ct, rows) {
  for (const row of rows) table(ct).push(row);
}

export function __rows(ct) {
  return [...table(ct)];
}

export function __emitted() {
  return [...emitted];
}

export function __seedReceipt(traceId, channelId, envelope) {
  receipts.set(String(traceId), { channel_id: channelId, envelope });
}

export function __seedChannel(channelKey, id) {
  channels.set(channelKey, { id });
}

// Configurable LLM reply (callApi output).
export let __llmReply = "mock reply";
export function __setLlReply(text) {
  __llmReply = text;
}
