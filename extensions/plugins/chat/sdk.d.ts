export interface DbExecResult {
    error?: string;
    rows_affected?: number;
}
export interface PluginError {
    __plugin_error: boolean;
    __status: number;
    __message: string;
}
export declare const SDK_VERSION: string;
export declare function dbPh(idx: number): string;
export declare function dbQuery(sql: string, params?: unknown[]): Record<string, unknown>[];
export declare function dbExec(sql: string, params?: unknown[]): DbExecResult;
export declare function dbBegin(): {
    ok: boolean;
};
export declare function dbCommit(): {
    ok: boolean;
};
export declare function dbRollback(): {
    ok: boolean;
};
export declare function httpGet(url: string): string | null;
export declare function httpGetJson(url: string): Record<string, unknown> | null;
export declare function httpPost(url: string, body: Record<string, unknown> | string): string | null;
export declare function httpPostJson(url: string, body: Record<string, unknown> | string): Record<string, unknown> | null;
export declare function configGet(key: string): string | null;
export declare function storeGet(key: string): string | null;
export declare function storeSet(key: string, value: string): boolean;
export declare function vfsRead(path: string): string | null;
export declare function vfsWrite(path: string, content: string): boolean;
export declare function vfsDelete(path: string): boolean;
export declare function vfsExists(path: string): boolean;
export declare function vfsList(path: string): string[] | null;
export declare function vfsStat(path: string): Record<string, unknown> | null;
export declare function getPost(slug: string): Record<string, unknown> | null;
export declare function ok(data: unknown): any;
export declare function fail(status: number, msg: string): PluginError;
export declare function extractJson(input: any, field?: string): any;
export declare function callApi(clientKey: string, op: string, input: unknown): Record<string, unknown>;
export declare function dbInsert(table: string, data: Record<string, unknown>, options?: Record<string, unknown>): {
    id?: string | number;
    rows_affected?: number;
    error?: string;
};
export declare function dbFetchOne(table: string, where?: Record<string, unknown>, options?: Record<string, unknown>): Record<string, unknown> | null;
export declare function dbFetchAll(table: string, where?: Record<string, unknown>, options?: Record<string, unknown>): Array<Record<string, unknown>>;
export declare function dbUpdate(table: string, data: Record<string, unknown>, where?: Record<string, unknown>, options?: Record<string, unknown>): {
    rows_affected?: number;
    error?: string;
};
export declare function dbDelete(table: string, where?: Record<string, unknown>, options?: Record<string, unknown>): {
    rows_affected?: number;
    error?: string;
};
export declare function dbCount(table: string, where?: Record<string, unknown>, options?: Record<string, unknown>): number;
export declare function dbIncrement(table: string, columns: Record<string, number>, where?: Record<string, unknown>, options?: Record<string, unknown>): {
    rows_affected?: number;
    error?: string;
};
export declare function dbSum(table: string, column: string, where?: Record<string, unknown>, options?: Record<string, unknown>): number;
export declare function dbGroupBy(table: string, options?: Record<string, unknown>): Array<Record<string, unknown>>;
export declare function ctFind(ct: string, query?: {
    filters?: Array<{
        field: string;
        op?: string;
        value: unknown;
    }>;
    page?: number;
    page_size?: number;
    sort?: string;
}): {
    rows: Array<Record<string, unknown>>;
    total: number;
};
export declare function ctGet(ct: string, id: string | number): Record<string, unknown> | null;
export declare function ctCreate(ct: string, data: Record<string, unknown>): Record<string, unknown>;
export declare function ctUpdate(ct: string, id: string | number, data: Record<string, unknown>): Record<string, unknown>;
export declare function jobEnqueue(jobType: string, payload: Record<string, unknown>, opts?: {
    max_attempts?: number;
    delay_secs?: number;
    delay_mins?: number;
}): {
    ok?: boolean;
};
export declare function getReceipt(traceId: string | number): Record<string, unknown> | null;
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
export declare function issueToken(input: IssueTokenInput): {
    token: string;
};
/** Verify a short-session widget JWT; returns claims or null (`session = ["verify"]`). */
export declare function verifyToken(token: string): VerifiedToken | null;
/**
 * Decode a base62-encoded (ID_ENCODING) snowflake id to its plain digit form.
 * On the plugin boundary PK ids are base62 while plain bigint FK fields are
 * digit strings — use this to compare a PK id against an FK or token claim.
 * Idempotent: already-digit ids pass through unchanged.
 */
export declare function decodeId(id: string | number): string;
/** Subjects currently available in a tenant (effective Online/Busy), as an
 * array of digit-string ids (`presence = ["available"]` permission). */
export declare function presenceAvailable(tenant: string): string[];
/** Effective presence status of one subject, e.g. "online"/"away"
 * (`presence = ["status"]` permission). */
export declare function presenceStatus(tenant: string, subject: string | number): string;
/** Set a subject's manual availability wish (away/busy/offline; empty/clear
 * clears it) (`presence = ["report"]` permission). */
export declare function presenceReport(tenant: string, subject: string | number, status?: string): void;
/** App-scoped channel host API (channel-app-ownership.md §4.2) — a plugin
 * manages only its own app's channels (`integration = ["channels"]`). */
/** List the invoking app's channels in the current tenant. */
export declare function channelList(): Channel[];
/** Create an app-owned channel; returns the created channel. `app_id` is
 * derived from the plugin id, never from the payload. */
export declare function channelCreate(data: Partial<ChannelInput> & {
    channel_key: string;
    provider: string;
    mode: string;
    transport: string;
    framing: string;
    codec: string;
    verify_kind: string;
    target_type: string;
}): Channel;
/** Partial-update an app-owned channel; returns the updated channel. */
export declare function channelUpdate(id: string | number, data: Partial<ChannelInput>): Channel;
/** Delete an app-owned channel. */
export declare function channelDelete(id: string | number): void;
/** Wire shape mirrors the kernel `CreateChannelRequest` (snake_case). */
export interface ChannelInput {
    channel_key: string;
    provider: string;
    display_name: string;
    mode: string;
    transport: string;
    framing: string;
    codec: string;
    endpoint?: string | null;
    verify_kind: string;
    verify_config?: unknown;
    credentials?: unknown;
    mapping?: unknown;
    pull_semantics?: string | null;
    pull_config?: unknown;
    stream_config?: unknown;
    redelivery_max?: number;
    backpressure?: unknown;
    target_type: string;
    route_extra?: unknown;
    enabled?: boolean;
}
/** Wire shape mirrors the kernel `ChannelResponse` (snake_case). */
export interface Channel {
    id: string;
    tenant_id: string;
    app_id: string | null;
    channel_key: string;
    provider: string;
    display_name: string;
    mode: string;
    transport: string;
    framing: string;
    codec: string;
    endpoint: string | null;
    verify_kind: string;
    verify_config?: unknown;
    mapping?: unknown;
    pull_semantics: string | null;
    pull_config?: unknown;
    stream_config?: unknown;
    ack_kind: string;
    redelivery_max: number;
    backpressure?: unknown;
    target_type: string;
    route_extra?: unknown;
    status: string;
    enabled: boolean;
    version: number;
    shadow: boolean;
    has_credentials: boolean;
}
export declare function logInfo(msg: string): void;
export declare function logWarn(msg: string): void;
export declare function logError(msg: string): void;
export declare function newId(): string;
export declare function eventEmit(type: string, data: string | Record<string, unknown>): void;
