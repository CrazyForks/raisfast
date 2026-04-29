/**
 * JS Plugin SDK v1 Type Declarations
 */
export const SDK_VERSION: string;

export function dbQuery(sql: string, params?: unknown[]): Record<string, unknown>[];
export function dbExec(sql: string, params?: unknown[]): { error?: string; rows_affected?: number };
export function dbBegin(): { ok: boolean };
export function dbCommit(): { ok: boolean };
export function dbRollback(): { ok: boolean };

export function httpGet(url: string): string | null;
export function httpGetJson(url: string): Record<string, unknown> | null;
export function httpPost(url: string, body: Record<string, unknown> | string): string | null;
export function httpPostJson(url: string, body: Record<string, unknown> | string): Record<string, unknown> | null;

export function configGet(key: string): string | null;
export function storeGet(key: string): string | null;
export function storeSet(key: string, value: string): boolean;

export function vfsRead(path: string): string | null;
export function vfsWrite(path: string, content: string): boolean;
export function vfsDelete(path: string): boolean;
export function vfsExists(path: string): boolean;
export function vfsList(path: string): string[] | null;

export function ok(data: unknown): any;
export function fail(status: number, msg: string): { __plugin_error: boolean; __status: number; __message: string };

export function extractJson(input: any, field?: string): any;

export function logInfo(msg: string): void;
export function logWarn(msg: string): void;
export function logError(msg: string): void;

export function newId(): string;
export function eventEmit(type: string, data: string | Record<string, unknown>): void;
