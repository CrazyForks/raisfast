// JS Plugin SDK v1 — 唯一真相源
// 由本文件生成 sdk.d.ts，分发到各插件目录
// Host 函数签名必须与 src/plugins/js_host.rs register_host_functions() 保持一致
export const SDK_VERSION = "1.0.0";
export function dbQuery(sql, params) {
    const result = RaisFastHost.dbQuery(sql, params ? JSON.stringify(params) : null);
    if (!result)
        throw new Error("query returned no result");
    if (result.startsWith("error:"))
        throw new Error(result.slice(6));
    return JSON.parse(result);
}
export function dbExec(sql, params) {
    const result = RaisFastHost.dbExecute(sql, params ? JSON.stringify(params) : null);
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
    return RaisFastHost.fsRead(path);
}
export function vfsWrite(path, content) {
    return RaisFastHost.fsWrite(path, content);
}
export function vfsDelete(path) {
    return RaisFastHost.fsDelete(path);
}
export function vfsExists(path) {
    return RaisFastHost.fsExists(path) ?? false;
}
export function vfsList(path) {
    const result = RaisFastHost.fsList(path);
    return result ? result.split(",") : null;
}
export function vfsStat(path) {
    const result = RaisFastHost.fsStat(path);
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
export function logInfo(msg) { RaisFastHost.log("info", msg); }
export function logWarn(msg) { RaisFastHost.log("warn", msg); }
export function logError(msg) { RaisFastHost.log("error", msg); }
export function newId() {
    return RaisFastHost.newId();
}
export function eventEmit(type, data) {
    RaisFastHost.emitEvent(type, typeof data === "string" ? data : JSON.stringify(data));
}
