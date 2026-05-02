export const SDK_VERSION = "1.0.0";

export function dbQuery(sql, params) {
    const result = Host.dbQuery(sql, params ? JSON.stringify(params) : null);
    if (!result) throw new Error("query returned no result");
    if (result.startsWith("error:")) throw new Error(result.slice(6));
    return JSON.parse(result);
}

export function dbExec(sql, params) {
    const result = Host.dbExecute(sql, params ? JSON.stringify(params) : null);
    return JSON.parse(result);
}

export function dbBegin() {
    const result = JSON.parse(Host.dbBegin());
    if (!result.ok) throw new Error("dbBegin failed");
    return result;
}
export function dbCommit() {
    const result = JSON.parse(Host.dbCommit());
    if (!result.ok) throw new Error("dbCommit failed");
    return result;
}
export function dbRollback() {
    return JSON.parse(Host.dbRollback());
}

export function httpGet(url) {
    return Host.httpGet(url);
}

export function httpGetJson(url) {
    const result = Host.httpGet(url);
    if (!result) return null;
    return JSON.parse(result);
}

export function httpPost(url, body) {
    const json = typeof body === "string" ? body : JSON.stringify(body);
    return Host.httpPost(url, json);
}

export function httpPostJson(url, body) {
    const json = typeof body === "string" ? body : JSON.stringify(body);
    const result = Host.httpPost(url, json);
    if (!result) return null;
    return JSON.parse(result);
}

export function configGet(key) { return Host.getConfig(key); }

export function storeGet(key) { return Host.getData(key); }
export function storeSet(key, value) { return Host.setData(key, value); }

export function vfsRead(path) { return Host.fsRead(path); }
export function vfsWrite(path, content) { return Host.fsWrite(path, content); }
export function vfsDelete(path) { return Host.fsDelete(path); }
export function vfsExists(path) { return Host.fsExists(path); }
export function vfsList(path) {
    const result = Host.fsList(path);
    return result ? result.split(",") : null;
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
        } else {
            parsed = input;
        }
        if (!field) return parsed;
        const parts = field.split(".");
        let val = parsed;
        for (const part of parts) {
            if (val == null || typeof val !== "object") return null;
            val = val[part];
        }
        if (typeof val === "string") {
            try { return JSON.parse(val); } catch { return val; }
        }
        return val != null ? val : null;
    } catch { return null; }
}

export function logInfo(msg) { Host.log("info", msg); }
export function logWarn(msg) { Host.log("warn", msg); }
export function logError(msg) { Host.log("error", msg); }

export function newId() { return Host.newId(); }

export function eventEmit(type, data) {
    return Host.emitEvent(type, typeof data === "string" ? data : JSON.stringify(data));
}
