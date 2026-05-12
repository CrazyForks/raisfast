import { SDKError } from "./errors";
import type {
  AfterSendHook,
  AuthResult,
  BeforeSendHook,
  IAuthStore,
  RequestOptions,
  SendOptions,
} from "./types";

export function toQueryString(query?: Record<string, string | number | bigint | boolean | undefined | null>): Record<string, string> | undefined {
  if (!query) return undefined;
  const result: Record<string, string> = {};
  for (const [key, value] of Object.entries(query)) {
    if (value !== undefined && value !== null) {
      result[key] = String(value);
    }
  }
  return result;
}

export class HttpClient {
  readonly baseUrl: string;
  readonly authStore: IAuthStore;
  private _tenantId: string | null = null;
  private _refreshPromise: Promise<string | null> | null = null;
  private _beforeSend: BeforeSendHook | null = null;
  private _afterSend: AfterSendHook | null = null;
  private _requestControllers = new Map<string, AbortController>();

  constructor(baseUrl: string, authStore: IAuthStore) {
    this.baseUrl = baseUrl;
    this.authStore = authStore;
  }

  get tenantId(): string | null {
    return this._tenantId;
  }

  setTenantId(tenantId: string | null): void {
    this._tenantId = tenantId;
  }

  set beforeSend(hook: BeforeSendHook | null) {
    this._beforeSend = hook;
  }

  set afterSend(hook: AfterSendHook | null) {
    this._afterSend = hook;
  }

  cancelRequest(key: string): void {
    this._requestControllers.get(key)?.abort();
    this._requestControllers.delete(key);
  }

  cancelAllRequests(): void {
    for (const controller of this._requestControllers.values()) {
      controller.abort();
    }
    this._requestControllers.clear();
  }

  async request<T>(path: string, options: SendOptions = {}): Promise<T> {
    const {
      method = "GET",
      body,
      query,
      headers: extraHeaders,
      signal,
      fetch: customFetch,
      requestKey,
    } = options;

    let url = `${this.baseUrl}${path}`;
    if (query) {
      const qs = new URLSearchParams(query).toString();
      if (qs) {
        url += (url.includes("?") ? "&" : "?") + qs;
      }
    }

    const headers = new Headers();
    if (body !== undefined && body !== null && !(body instanceof FormData)) {
      headers.set("Content-Type", "application/json");
    }
    const token = this.authStore.token;
    if (token) {
      headers.set("Authorization", `Bearer ${token}`);
    }
    if (this._tenantId) {
      headers.set("X-Tenant-ID", this._tenantId);
    }
    if (extraHeaders) {
      for (const [key, value] of Object.entries(extraHeaders)) {
        headers.set(key, value);
      }
    }

    let combinedSignal = signal;
    if (requestKey) {
      this.cancelRequest(requestKey);
      const controller = new AbortController();
      this._requestControllers.set(requestKey, controller);
      if (signal) {
        signal.addEventListener("abort", () => controller.abort());
      }
      combinedSignal = controller.signal;
    }

    let fetchOptions: RequestInit = {
      method,
      headers,
      body:
        body instanceof FormData
          ? body
          : body !== undefined
            ? JSON.stringify(body)
            : undefined,
      signal: combinedSignal,
    };

    if (this._beforeSend) {
      const result = await this._beforeSend(url, fetchOptions);
      url = result.url;
      fetchOptions = { ...fetchOptions, ...result.options };
    }

    const fetchFn = customFetch ?? fetch;
    let res: Response;

    try {
      res = await fetchFn(url, fetchOptions);
    } catch (e) {
      if (requestKey) this._requestControllers.delete(requestKey);
      throw new SDKError(
        0,
        e instanceof Error ? e.message : "Network request failed",
        0,
        url,
        {},
        e instanceof DOMException && e.name === "AbortError",
        e instanceof Error ? e : null,
      );
    }

    if (res.status === 401 && this.authStore.isAuthenticated) {
      const newToken = await this._refresh();
      if (newToken) {
        const retryHeaders = new Headers(fetchOptions.headers as Headers);
        retryHeaders.set("Authorization", `Bearer ${newToken}`);
        fetchOptions = { ...fetchOptions, headers: retryHeaders };
        res = await fetchFn(url, fetchOptions);
      }
    }

    if (requestKey) this._requestControllers.delete(requestKey);

    let json: Record<string, unknown>;
    try {
      json = (await res.json()) as Record<string, unknown>;
    } catch (e) {
      throw new SDKError(
        0,
        "Failed to parse response",
        res.status,
        url,
        {},
        false,
        e instanceof Error ? e : null,
      );
    }

    if (json.code !== 0) {
      throw new SDKError(
        (json.code as number) ?? 0,
        (json.message as string) ?? "Unknown error",
        res.status,
        url,
        json,
      );
    }

    let data = json.data as T;
    if (this._afterSend) {
      data = (await this._afterSend(res, data)) as T;
    }

    return data;
  }

  async get<T>(path: string, options?: RequestOptions): Promise<T> {
    return this.request<T>(path, { ...options, method: "GET" });
  }

  async post<T>(
    path: string,
    body?: unknown,
    options?: RequestOptions,
  ): Promise<T> {
    return this.request<T>(path, { ...options, method: "POST", body });
  }

  async put<T>(
    path: string,
    body?: unknown,
    options?: RequestOptions,
  ): Promise<T> {
    return this.request<T>(path, { ...options, method: "PUT", body });
  }

  async del<T>(path: string, options?: RequestOptions): Promise<T> {
    return this.request<T>(path, { ...options, method: "DELETE" });
  }

  private async _refresh(): Promise<string | null> {
    if (this._refreshPromise) return this._refreshPromise;

    this._refreshPromise = (async () => {
      try {
        const rt = this.authStore.refreshToken;
        if (!rt) return null;

        const res = await fetch(`${this.baseUrl}/auth/refresh`, {
          method: "POST",
          headers: { "Content-Type": "application/json" },
          body: JSON.stringify({ refresh_token: rt }),
        });

        const json = (await res.json()) as Record<string, unknown>;
        if (json.code !== 0) {
          this.authStore.clear();
          return null;
        }

        const auth = json.data as AuthResult;
        this.authStore.save(auth);
        return auth.access_token;
      } catch {
        this.authStore.clear();
        return null;
      } finally {
        this._refreshPromise = null;
      }
    })();

    return this._refreshPromise;
  }
}
