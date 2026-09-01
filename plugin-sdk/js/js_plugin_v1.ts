// JS Plugin SDK v1 — 唯一真相源
// 由本文件生成 sdk.d.ts，分发到各插件目录
// Host 函数签名必须与 src/plugins/js_host.rs register_host_functions() 保持一致

/* eslint-disable @typescript-eslint/no-explicit-any */

declare const RaisFastHost: {
  log(level: string, msg: string): void;
  httpGet(url: string): string;
  httpPost(url: string, body: string): string;
  callApi(clientKey: string, op: string, input: string): string;
  ctFind(ct: string, query: string): string;
  ctGet(ct: string, id: string): string | null;
  ctCreate(ct: string, data: string): string;
  ctUpdate(ct: string, id: string, data: string): string;
  jobEnqueue(jobType: string, payload: string, opts: string): string;
  getReceipt(traceId: string): string | null;
  issueToken(input: string): string;
  verifyToken(token: string): string | null;
  decodeId(id: string): string;
  presenceAvailable(tenant: string): string;
  presenceStatus(tenant: string, subject: string): string;
  presenceReport(tenant: string, subject: string, status: string): string;
  dbInsert(table: string, data: string, options: string): string;
  dbFetchOne(table: string, where: string, options: string): string;
  dbFetchAll(table: string, where: string, options: string): string;
  dbUpdate(table: string, data: string, where: string, options: string): string;
  dbDelete(table: string, where: string, options: string): string;
  dbCount(table: string, where: string, options: string): string;
  dbIncrement(table: string, columns: string, where: string, options: string): string;
  dbSum(table: string, column: string, where: string, options: string): string;
  dbGroupBy(table: string, options: string): string;
  getConfig(key: string): string | null;
  httpGet(url: string): string;
  httpPost(url: string, body: string): string;
  getData(key: string): string | null;
  setData(key: string, value: string): boolean;
  getPost(slug: string): string | null;
  dbPh(idx: number): string;
  dbQuery(sql: string, params: string): string;
  dbExecute(sql: string, params: string): string;
  dbBegin(): string;
  dbCommit(): string;
  dbRollback(): string;
  vfsRead(path: string): string | null;
  vfsWrite(path: string, content: string): boolean;
  vfsDelete(path: string): boolean;
  vfsExists(path: string): boolean | null;
  vfsList(path: string): string | null;
  vfsStat(path: string): string | null;
  newId(): string;
  emitEvent(eventType: string, data: string): string;
};

export interface DbExecResult {
  error?: string;
  rows_affected?: number;
}

export interface PluginError {
  __plugin_error: boolean;
  __status: number;
  __message: string;
}

export const SDK_VERSION: string = "1.0.0";

export function dbPh(idx: number): string {
  return RaisFastHost.dbPh(idx);
}

export function dbQuery(sql: string, params: unknown[] = []): Record<string, unknown>[] {
  const result = RaisFastHost.dbQuery(sql, JSON.stringify(params));
  if (!result) throw new Error("query returned no result");
  if (result.startsWith("error:")) throw new Error(result.slice(6));
  return JSON.parse(result);
}

export function dbExec(sql: string, params: unknown[] = []): DbExecResult {
  const result = RaisFastHost.dbExecute(sql, JSON.stringify(params));
  return JSON.parse(result);
}

export function dbBegin(): { ok: boolean } {
  const result = JSON.parse(RaisFastHost.dbBegin());
  if (!result.ok) throw new Error("dbBegin failed");
  return result;
}

export function dbCommit(): { ok: boolean } {
  const result = JSON.parse(RaisFastHost.dbCommit());
  if (!result.ok) throw new Error("dbCommit failed");
  return result;
}

export function dbRollback(): { ok: boolean } {
  return JSON.parse(RaisFastHost.dbRollback());
}

export function httpGet(url: string): string | null {
  return RaisFastHost.httpGet(url) || null;
}

export function httpGetJson(url: string): Record<string, unknown> | null {
  const result = RaisFastHost.httpGet(url);
  if (!result) return null;
  return JSON.parse(result);
}

export function httpPost(url: string, body: Record<string, unknown> | string): string | null {
  const json = typeof body === "string" ? body : JSON.stringify(body);
  return RaisFastHost.httpPost(url, json) || null;
}

export function httpPostJson(url: string, body: Record<string, unknown> | string): Record<string, unknown> | null {
  const json = typeof body === "string" ? body : JSON.stringify(body);
  const result = RaisFastHost.httpPost(url, json);
  if (!result) return null;
  return JSON.parse(result);
}

export function configGet(key: string): string | null {
  return RaisFastHost.getConfig(key);
}

export function storeGet(key: string): string | null {
  return RaisFastHost.getData(key);
}

export function storeSet(key: string, value: string): boolean {
  return RaisFastHost.setData(key, value);
}

export function vfsRead(path: string): string | null {
  return RaisFastHost.vfsRead(path);
}

export function vfsWrite(path: string, content: string): boolean {
  return RaisFastHost.vfsWrite(path, content);
}

export function vfsDelete(path: string): boolean {
  return RaisFastHost.vfsDelete(path);
}

export function vfsExists(path: string): boolean {
  return RaisFastHost.vfsExists(path) ?? false;
}

export function vfsList(path: string): string[] | null {
  const result = RaisFastHost.vfsList(path);
  return result ? result.split(",") : null;
}

export function vfsStat(path: string): Record<string, unknown> | null {
  const result = RaisFastHost.vfsStat(path);
  return result ? JSON.parse(result) : null;
}

export function getPost(slug: string): Record<string, unknown> | null {
  const result = RaisFastHost.getPost(slug);
  return result ? JSON.parse(result) : null;
}

export function ok(data: unknown): any {
  return data;
}

export function fail(status: number, msg: string): PluginError {
  return { __plugin_error: true, __status: status, __message: msg };
}

export function extractJson(input: any, field?: string): any {
  try {
    let parsed: any;
    if (typeof input === "string") {
      parsed = JSON.parse(input);
    } else {
      parsed = input;
    }
    if (!field) return parsed;
    const parts = field.split(".");
    let val: any = parsed;
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


export function callApi(clientKey: string, op: string, input: unknown): Record<string, unknown> {
  const result = RaisFastHost.callApi(clientKey, op, JSON.stringify(input ?? {}));
  return JSON.parse(result);
}
export function dbInsert(table: string, data: Record<string, unknown>, options?: Record<string, unknown>): { id?: string | number; rows_affected?: number; error?: string } {
  const result = JSON.parse(RaisFastHost.dbInsert(table, JSON.stringify(data ?? {}), JSON.stringify(options ?? {})));
  if (result.error) throw new Error(result.error);
  return result;
}
export function dbFetchOne(table: string, where?: Record<string, unknown>, options?: Record<string, unknown>): Record<string, unknown> | null {
  const result = JSON.parse(RaisFastHost.dbFetchOne(table, JSON.stringify(where ?? {}), JSON.stringify(options ?? {})));
  if (result.error) throw new Error(result.error);
  return result.row ?? null;
}
export function dbFetchAll(table: string, where?: Record<string, unknown>, options?: Record<string, unknown>): Array<Record<string, unknown>> {
  const result = JSON.parse(RaisFastHost.dbFetchAll(table, JSON.stringify(where ?? {}), JSON.stringify(options ?? {})));
  if (result.error) throw new Error(result.error);
  return result.rows ?? [];
}
export function dbUpdate(table: string, data: Record<string, unknown>, where?: Record<string, unknown>, options?: Record<string, unknown>): { rows_affected?: number; error?: string } {
  const result = JSON.parse(RaisFastHost.dbUpdate(table, JSON.stringify(data ?? {}), JSON.stringify(where ?? {}), JSON.stringify(options ?? {})));
  if (result.error) throw new Error(result.error);
  return result;
}
export function dbDelete(table: string, where?: Record<string, unknown>, options?: Record<string, unknown>): { rows_affected?: number; error?: string } {
  const result = JSON.parse(RaisFastHost.dbDelete(table, JSON.stringify(where ?? {}), JSON.stringify(options ?? {})));
  if (result.error) throw new Error(result.error);
  return result;
}
export function dbCount(table: string, where?: Record<string, unknown>, options?: Record<string, unknown>): number {
  const result = JSON.parse(RaisFastHost.dbCount(table, JSON.stringify(where ?? {}), JSON.stringify(options ?? {})));
  if (result.error) throw new Error(result.error);
  return result.count ?? 0;
}
export function dbIncrement(table: string, columns: Record<string, number>, where?: Record<string, unknown>, options?: Record<string, unknown>): { rows_affected?: number; error?: string } {
  const result = JSON.parse(RaisFastHost.dbIncrement(table, JSON.stringify(columns ?? {}), JSON.stringify(where ?? {}), JSON.stringify(options ?? {})));
  if (result.error) throw new Error(result.error);
  return result;
}
export function dbSum(table: string, column: string, where?: Record<string, unknown>, options?: Record<string, unknown>): number {
  const result = JSON.parse(RaisFastHost.dbSum(table, column, JSON.stringify(where ?? {}), JSON.stringify(options ?? {})));
  if (result.error) throw new Error(result.error);
  return result.sum ?? 0;
}
export function dbGroupBy(table: string, options?: Record<string, unknown>): Array<Record<string, unknown>> {
  const result = JSON.parse(RaisFastHost.dbGroupBy(table, JSON.stringify(options ?? {})));
  if (result.error) throw new Error(result.error);
  return result.rows ?? [];
}

// ── Content-type host API (group-aware: 'group/plural', plural, table) ───
export function ctFind(ct: string, query?: {
    filters?: Array<{ field: string; op?: string; value: unknown }>;
    page?: number;
    page_size?: number;
    sort?: string;
}): { rows: Array<Record<string, unknown>>; total: number } {
    const result = RaisFastHost.ctFind(ct, JSON.stringify(query ?? {}));
    const parsed = JSON.parse(result);
    if (parsed.error) throw new Error(parsed.error);
    return parsed;
}
export function ctGet(ct: string, id: string | number): Record<string, unknown> | null {
    const result = RaisFastHost.ctGet(ct, String(id));
    if (result === null || result === "null") return null;
    return JSON.parse(result);
}
export function ctCreate(ct: string, data: Record<string, unknown>): Record<string, unknown> {
    const result = RaisFastHost.ctCreate(ct, JSON.stringify(data ?? {}));
    const parsed = JSON.parse(result);
    if (parsed && parsed.error) throw new Error(parsed.error);
    return parsed;
}
export function ctUpdate(ct: string, id: string | number, data: Record<string, unknown>): Record<string, unknown> {
    const result = RaisFastHost.ctUpdate(ct, String(id), JSON.stringify(data ?? {}));
    const parsed = JSON.parse(result);
    if (parsed && parsed.error) throw new Error(parsed.error);
    return parsed;
}
// ── Job / integration host API ──────────────────────────────
export function jobEnqueue(jobType: string, payload: Record<string, unknown>, opts?: {
    max_attempts?: number;
    delay_secs?: number;
    delay_mins?: number;
}): { ok?: boolean } {
    const result = RaisFastHost.jobEnqueue(jobType, JSON.stringify(payload ?? {}), JSON.stringify(opts ?? {}));
    const parsed = JSON.parse(result);
    if (parsed.error) throw new Error(parsed.error);
    return parsed;
}
export function getReceipt(traceId: string | number): Record<string, unknown> | null {
    const result = RaisFastHost.getReceipt(String(traceId));
    if (result === null || result === "null") return null;
    return JSON.parse(result);
}

export interface IssueTokenInput {
    channel_key: string;
    contact_id: string;
    ttl_secs?: number;
}

export interface VerifiedToken {
    channel_key: string;
    contact_id: string;
}

/** Sign a short-session widget JWT (`session = ["issue"]` permission). */
export function issueToken(input: IssueTokenInput): { token: string } {
    const result = RaisFastHost.issueToken(JSON.stringify(input));
    const parsed = JSON.parse(result);
    if (parsed?.error) throw new Error(parsed.error);
    return parsed;
}

/** Verify a short-session widget JWT; returns claims or null (`session = ["verify"]`). */
export function verifyToken(token: string): VerifiedToken | null {
    const result = RaisFastHost.verifyToken(String(token));
    if (result === null || result === "null") return null;
    const parsed = JSON.parse(result);
    if (parsed?.error) throw new Error(parsed.error);
    return parsed;
}

/**
 * Decode a base62-encoded (ID_ENCODING) snowflake id to its plain digit form.
 * On the plugin boundary PK ids are base62 while plain bigint FK fields are
 * digit strings — use this to compare a PK id against an FK or token claim.
 * Idempotent: already-digit ids pass through unchanged.
 */
export function decodeId(id: string | number): string {
    return RaisFastHost.decodeId(String(id));
}

/** Subjects currently available in a tenant (effective Online/Busy), as an
 * array of digit-string ids (`presence = ["available"]` permission). */
export function presenceAvailable(tenant: string): string[] {
    const result = RaisFastHost.presenceAvailable(String(tenant));
    const parsed = JSON.parse(result);
    if (parsed?.error) throw new Error(parsed.error);
    return parsed;
}

/** Effective presence status of one subject, e.g. "online"/"away"
 * (`presence = ["status"]` permission). */
export function presenceStatus(tenant: string, subject: string | number): string {
    const result = RaisFastHost.presenceStatus(String(tenant), String(subject));
    const parsed = JSON.parse(result);
    if (parsed?.error) throw new Error(parsed.error);
    return parsed;
}

/** Set a subject's manual availability wish (away/busy/offline; empty/clear
 * clears it) (`presence = ["report"]` permission). */
export function presenceReport(tenant: string, subject: string | number, status?: string): void {
    const result = RaisFastHost.presenceReport(String(tenant), String(subject), String(status ?? ""));
    const parsed = JSON.parse(result);
    if (parsed?.error) throw new Error(parsed.error);
}

export function logInfo(msg: string): void { RaisFastHost.log("info", msg); }
export function logWarn(msg: string): void { RaisFastHost.log("warn", msg); }
export function logError(msg: string): void { RaisFastHost.log("error", msg); }

export function newId(): string {
  return RaisFastHost.newId();
}

export function eventEmit(type: string, data: string | Record<string, unknown>): void {
  RaisFastHost.emitEvent(type, typeof data === "string" ? data : JSON.stringify(data));
}
