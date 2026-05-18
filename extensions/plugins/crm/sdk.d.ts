export interface DbExecResult {
    error?: string;
    rows_affected?: number;
}
export interface DbCrudResult {
    rows_affected?: number;
}
export interface DbFetchAllResult {
    data: Record<string, unknown>[];
    total: number;
}
export interface DbGroupByResult {
    data: Record<string, unknown>[];
    total: number;
}
export interface PluginError {
    __plugin_error: boolean;
    __status: number;
    __message: string;
}
export declare const SDK_VERSION: string;
export declare function dbQuery(sql: string, params?: unknown[]): Record<string, unknown>[];
export declare function dbExec(sql: string, params?: unknown[]): DbExecResult;
export declare function dbInsert(table: string, data: Record<string, unknown> | string, options?: Record<string, unknown>): DbCrudResult;
export declare function dbFetchOne(table: string, where: Record<string, unknown> | string | unknown[], options?: Record<string, unknown>): Record<string, unknown> | null;
export declare function dbFetchAll(table: string, where?: Record<string, unknown> | string | unknown[] | null, options?: Record<string, unknown>): DbFetchAllResult;
export declare function dbUpdate(table: string, data: Record<string, unknown> | string, where: Record<string, unknown> | string | unknown[], options?: Record<string, unknown>): DbCrudResult;
export declare function dbDelete(table: string, where: Record<string, unknown> | string | unknown[], options?: Record<string, unknown>): DbCrudResult;
export declare function dbCount(table: string, where?: Record<string, unknown> | string | unknown[] | null, options?: Record<string, unknown>): number;
export declare function dbIncrement(table: string, columns: Record<string, number> | string, where?: Record<string, unknown> | string | unknown[] | null, options?: Record<string, unknown>): DbCrudResult;
export declare function dbSum(table: string, column: string, where?: Record<string, unknown> | string | unknown[] | null, options?: Record<string, unknown>): number;
export declare function dbGroupBy(table: string, options: Record<string, unknown> | string): DbGroupByResult;
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
export declare function logInfo(msg: string): void;
export declare function logWarn(msg: string): void;
export declare function logError(msg: string): void;
export declare function newId(): string;
export declare function eventEmit(type: string, data: string | Record<string, unknown>): void;
