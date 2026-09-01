// JS Plugin SDK v1 — 唯一真相源
// 由本文件生成 sdk.d.ts，分发到各插件目录
// Host 函数签名必须与 src/plugins/js_host.rs register_host_functions() 保持一致
export const SDK_VERSION = "1.0.0";
export function dbPh(idx) {
    return RaisFastHost.dbPh(idx);
}
export function dbQuery(sql, params = []) {
    const result = RaisFastHost.dbQuery(sql, JSON.stringify(params));
    if (!result)
        throw new Error("query returned no result");
    if (result.startsWith("error:"))
        throw new Error(result.slice(6));
    return JSON.parse(result);
}
export function dbExec(sql, params = []) {
    const result = RaisFastHost.dbExecute(sql, JSON.stringify(params));
    return JSON.parse(result);
}
export function dbBegin() {
    const result = JSON.parse(RaisFastHost.dbBegin());
    if (!result.ok)
        throw new Error("dbBegin failed");
    return result;
}
export function dbCommit() {
    const result = JSON.parse(RaisFastHost.dbCommit());
    if (!result.ok)
        throw new Error("dbCommit failed");
    return result;
}
export function dbRollback() {
    return JSON.parse(RaisFastHost.dbRollback());
}
export function httpGet(url) {
    return RaisFastHost.httpGet(url) || null;
}
export function httpGetJson(url) {
    const result = RaisFastHost.httpGet(url);
    if (!result)
        return null;
    return JSON.parse(result);
}
export function httpPost(url, body) {
    const json = typeof body === "string" ? body : JSON.stringify(body);
    return RaisFastHost.httpPost(url, json) || null;
}
export function httpPostJson(url, body) {
    const json = typeof body === "string" ? body : JSON.stringify(body);
    const result = RaisFastHost.httpPost(url, json);
    if (!result)
        return null;
    return JSON.parse(result);
}
export function configGet(key) {
    return RaisFastHost.getConfig(key);
}
export function storeGet(key) {
    return RaisFastHost.getData(key);
}
export function storeSet(key, value) {
    return RaisFastHost.setData(key, value);
}
export function vfsRead(path) {
    return RaisFastHost.vfsRead(path);
}
export function vfsWrite(path, content) {
    return RaisFastHost.vfsWrite(path, content);
}
export function vfsDelete(path) {
    return RaisFastHost.vfsDelete(path);
}
export function vfsExists(path) {
    return RaisFastHost.vfsExists(path) ?? false;
}
export function vfsList(path) {
    const result = RaisFastHost.vfsList(path);
    return result ? result.split(",") : null;
}
export function vfsStat(path) {
    const result = RaisFastHost.vfsStat(path);
    return result ? JSON.parse(result) : null;
}
export function getPost(slug) {
    const result = RaisFastHost.getPost(slug);
    return result ? JSON.parse(result) : null;
}
export function ok(data) {
    return data;
}
export function fail(status, msg) {
    return { __plugin_error: true, __status: status, __message: msg };
}
export function extractJson(input, field) {
    try {
        let parsed;
        if (typeof input === "string") {
            parsed = JSON.parse(input);
        }
        else {
            parsed = input;
        }
        if (!field)
            return parsed;
        const parts = field.split(".");
        let val = parsed;
        for (const part of parts) {
            if (val == null || typeof val !== "object")
                return null;
            val = val[part];
        }
        if (typeof val === "string") {
            try {
                return JSON.parse(val);
            }
            catch {
                return val;
            }
        }
        return val != null ? val : null;
    }
    catch {
        return null;
    }
}
export function callApi(clientKey, op, input) {
    const result = RaisFastHost.callApi(clientKey, op, JSON.stringify(input ?? {}));
    return JSON.parse(result);
}
export function dbInsert(table, data, options) {
    const result = JSON.parse(RaisFastHost.dbInsert(table, JSON.stringify(data ?? {}), JSON.stringify(options ?? {})));
    if (result.error)
        throw new Error(result.error);
    return result;
}
export function dbFetchOne(table, where, options) {
    const result = JSON.parse(RaisFastHost.dbFetchOne(table, JSON.stringify(where ?? {}), JSON.stringify(options ?? {})));
    if (result.error)
        throw new Error(result.error);
    return result.row ?? null;
}
export function dbFetchAll(table, where, options) {
    const result = JSON.parse(RaisFastHost.dbFetchAll(table, JSON.stringify(where ?? {}), JSON.stringify(options ?? {})));
    if (result.error)
        throw new Error(result.error);
    return result.rows ?? [];
}
export function dbUpdate(table, data, where, options) {
    const result = JSON.parse(RaisFastHost.dbUpdate(table, JSON.stringify(data ?? {}), JSON.stringify(where ?? {}), JSON.stringify(options ?? {})));
    if (result.error)
        throw new Error(result.error);
    return result;
}
export function dbDelete(table, where, options) {
    const result = JSON.parse(RaisFastHost.dbDelete(table, JSON.stringify(where ?? {}), JSON.stringify(options ?? {})));
    if (result.error)
        throw new Error(result.error);
    return result;
}
export function dbCount(table, where, options) {
    const result = JSON.parse(RaisFastHost.dbCount(table, JSON.stringify(where ?? {}), JSON.stringify(options ?? {})));
    if (result.error)
        throw new Error(result.error);
    return result.count ?? 0;
}
export function dbIncrement(table, columns, where, options) {
    const result = JSON.parse(RaisFastHost.dbIncrement(table, JSON.stringify(columns ?? {}), JSON.stringify(where ?? {}), JSON.stringify(options ?? {})));
    if (result.error)
        throw new Error(result.error);
    return result;
}
export function dbSum(table, column, where, options) {
    const result = JSON.parse(RaisFastHost.dbSum(table, column, JSON.stringify(where ?? {}), JSON.stringify(options ?? {})));
    if (result.error)
        throw new Error(result.error);
    return result.sum ?? 0;
}
export function dbGroupBy(table, options) {
    const result = JSON.parse(RaisFastHost.dbGroupBy(table, JSON.stringify(options ?? {})));
    if (result.error)
        throw new Error(result.error);
    return result.rows ?? [];
}
// ── Content-type host API (group-aware: 'group/plural', plural, table) ───
export function ctFind(ct, query) {
    const result = RaisFastHost.ctFind(ct, JSON.stringify(query ?? {}));
    const parsed = JSON.parse(result);
    if (parsed.error)
        throw new Error(parsed.error);
    return parsed;
}
export function ctGet(ct, id) {
    const result = RaisFastHost.ctGet(ct, String(id));
    if (result === null || result === "null")
        return null;
    return JSON.parse(result);
}
export function ctCreate(ct, data) {
    const result = RaisFastHost.ctCreate(ct, JSON.stringify(data ?? {}));
    const parsed = JSON.parse(result);
    if (parsed && parsed.error)
        throw new Error(parsed.error);
    return parsed;
}
export function ctUpdate(ct, id, data) {
    const result = RaisFastHost.ctUpdate(ct, String(id), JSON.stringify(data ?? {}));
    const parsed = JSON.parse(result);
    if (parsed && parsed.error)
        throw new Error(parsed.error);
    return parsed;
}
// ── Job / integration host API ──────────────────────────────
export function jobEnqueue(jobType, payload, opts) {
    const result = RaisFastHost.jobEnqueue(jobType, JSON.stringify(payload ?? {}), JSON.stringify(opts ?? {}));
    const parsed = JSON.parse(result);
    if (parsed.error)
        throw new Error(parsed.error);
    return parsed;
}
export function getReceipt(traceId) {
    const result = RaisFastHost.getReceipt(String(traceId));
    if (result === null || result === "null")
        return null;
    return JSON.parse(result);
}
/** Sign a short-session widget JWT (`session = ["issue"]` permission). */
export function issueToken(input) {
    const result = RaisFastHost.issueToken(JSON.stringify(input));
    const parsed = JSON.parse(result);
    if (parsed?.error)
        throw new Error(parsed.error);
    return parsed;
}
/** Verify a short-session widget JWT; returns claims or null (`session = ["verify"]`). */
export function verifyToken(token) {
    const result = RaisFastHost.verifyToken(String(token));
    if (result === null || result === "null")
        return null;
    const parsed = JSON.parse(result);
    if (parsed?.error)
        throw new Error(parsed.error);
    return parsed;
}
/**
 * Decode a base62-encoded (ID_ENCODING) snowflake id to its plain digit form.
 * On the plugin boundary PK ids are base62 while plain bigint FK fields are
 * digit strings — use this to compare a PK id against an FK or token claim.
 * Idempotent: already-digit ids pass through unchanged.
 */
export function decodeId(id) {
    return RaisFastHost.decodeId(String(id));
}
/** Subjects currently available in a tenant (effective Online/Busy), as an
 * array of digit-string ids (`presence = ["available"]` permission). */
export function presenceAvailable(tenant) {
    const result = RaisFastHost.presenceAvailable(String(tenant));
    const parsed = JSON.parse(result);
    if (parsed?.error)
        throw new Error(parsed.error);
    return parsed;
}
/** Effective presence status of one subject, e.g. "online"/"away"
 * (`presence = ["status"]` permission). */
export function presenceStatus(tenant, subject) {
    const result = RaisFastHost.presenceStatus(String(tenant), String(subject));
    const parsed = JSON.parse(result);
    if (parsed?.error)
        throw new Error(parsed.error);
    return parsed;
}
/** Set a subject's manual availability wish (away/busy/offline; empty/clear
 * clears it) (`presence = ["report"]` permission). */
export function presenceReport(tenant, subject, status) {
    const result = RaisFastHost.presenceReport(String(tenant), String(subject), String(status ?? ""));
    const parsed = JSON.parse(result);
    if (parsed?.error)
        throw new Error(parsed.error);
}
export function logInfo(msg) { RaisFastHost.log("info", msg); }
export function logWarn(msg) { RaisFastHost.log("warn", msg); }
export function logError(msg) { RaisFastHost.log("error", msg); }
export function newId() {
    return RaisFastHost.newId();
}
export function eventEmit(type, data) {
    RaisFastHost.emitEvent(type, typeof data === "string" ? data : JSON.stringify(data));
}
