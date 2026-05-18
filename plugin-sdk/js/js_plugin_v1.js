// JS Plugin SDK v1 — 唯一真相源
// 由本文件生成 sdk.d.ts，分发到各插件目录
// Host 函数签名必须与 src/plugins/js_host.rs register_host_functions() 保持一致
export const SDK_VERSION = "1.0.0";
export function dbPh(idx) {
    return RaisFastHost.dbPh(idx);
}
export function dbQuery(sql, params) {
    const result = RaisFastHost.dbQuery(sql, JSON.stringify(params ?? []));
    if (!result)
        throw new Error("query returned no result");
    if (result.startsWith("error:"))
        throw new Error(result.slice(6));
    return JSON.parse(result);
}
export function dbExec(sql, params) {
    const result = RaisFastHost.dbExecute(sql, JSON.stringify(params ?? []));
    return JSON.parse(result);
}
export function dbInsert(table, data, options) {
    const dataStr = typeof data === "string" ? data : JSON.stringify(data);
    const optStr = options == null ? "{}" : (typeof options === "string" ? options : JSON.stringify(options));
    const result = JSON.parse(RaisFastHost.dbInsert(table, dataStr, optStr));
    if (result.error) throw new Error(result.error);
    return result;
}
export function dbFetchOne(table, where, options) {
    const whereStr = where == null ? "{}" : (typeof where === "string" ? where : JSON.stringify(where));
    const optStr = options == null ? "{}" : (typeof options === "string" ? options : JSON.stringify(options));
    const result = JSON.parse(RaisFastHost.dbFetchOne(table, whereStr, optStr));
    if (result.error) throw new Error(result.error);
    return result.data;
}
export function dbFetchAll(table, where, options) {
    const whereStr = where == null ? "{}" : (typeof where === "string" ? where : JSON.stringify(where));
    const optStr = options == null ? "{}" : (typeof options === "string" ? options : JSON.stringify(options));
    const result = JSON.parse(RaisFastHost.dbFetchAll(table, whereStr, optStr));
    if (result.error) throw new Error(result.error);
    return result;
}
export function dbUpdate(table, data, where, options) {
    const dataStr = typeof data === "string" ? data : JSON.stringify(data);
    const whereStr = where == null ? "{}" : (typeof where === "string" ? where : JSON.stringify(where));
    const optStr = options == null ? "{}" : (typeof options === "string" ? options : JSON.stringify(options));
    const result = JSON.parse(RaisFastHost.dbUpdate(table, dataStr, whereStr, optStr));
    if (result.error) throw new Error(result.error);
    return result;
}
export function dbDelete(table, where, options) {
    const whereStr = where == null ? "{}" : (typeof where === "string" ? where : JSON.stringify(where));
    const optStr = options == null ? "{}" : (typeof options === "string" ? options : JSON.stringify(options));
    const result = JSON.parse(RaisFastHost.dbDelete(table, whereStr, optStr));
    if (result.error) throw new Error(result.error);
    return result;
}
export function dbCount(table, where, options) {
    const whereStr = where == null ? "{}" : (typeof where === "string" ? where : JSON.stringify(where));
    const optStr = options == null ? "{}" : (typeof options === "string" ? options : JSON.stringify(options));
    const result = JSON.parse(RaisFastHost.dbCount(table, whereStr, optStr));
    if (result.error) throw new Error(result.error);
    return result.count;
}
export function dbIncrement(table, columns, where, options) {
    const colStr = typeof columns === "string" ? columns : JSON.stringify(columns);
    const whereStr = where == null ? "{}" : (typeof where === "string" ? where : JSON.stringify(where));
    const optStr = options == null ? "{}" : (typeof options === "string" ? options : JSON.stringify(options));
    const result = JSON.parse(RaisFastHost.dbIncrement(table, colStr, whereStr, optStr));
    if (result.error) throw new Error(result.error);
    return result;
}
export function dbSum(table, column, where, options) {
    const whereStr = where == null ? "{}" : (typeof where === "string" ? where : JSON.stringify(where));
    const optStr = options == null ? "{}" : (typeof options === "string" ? options : JSON.stringify(options));
    const result = JSON.parse(RaisFastHost.dbSum(table, column, whereStr, optStr));
    if (result.error) throw new Error(result.error);
    return result.sum;
}
export function dbGroupBy(table, options) {
    const optStr = options == null ? "{}" : (typeof options === "string" ? options : JSON.stringify(options));
    const result = JSON.parse(RaisFastHost.dbGroupBy(table, optStr));
    if (result.error) throw new Error(result.error);
    return result;
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
export function logInfo(msg) { RaisFastHost.log("info", msg); }
export function logWarn(msg) { RaisFastHost.log("warn", msg); }
export function logError(msg) { RaisFastHost.log("error", msg); }
export function newId() {
    return RaisFastHost.newId();
}
export function eventEmit(type, data) {
    RaisFastHost.emitEvent(type, typeof data === "string" ? data : JSON.stringify(data));
}
