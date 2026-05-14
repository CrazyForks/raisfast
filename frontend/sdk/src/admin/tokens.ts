import { HttpClient } from "../client";
import type { ApiToken, RequestOptions } from "../types";

export class AdminTokens {
  private readonly http: HttpClient;

  constructor(http: HttpClient) {
    this.http = http;
  }

  async list(options?: RequestOptions): Promise<ApiToken[]> {
    return this.http.get<ApiToken[]>("/tokens", options);
  }

  async create(
    data: { name: string; scopes: string[]; expires_at?: string },
    options?: RequestOptions,
  ): Promise<ApiToken & { token: string }> {
    return this.http.request<ApiToken & { token: string }>(this.http.pathForCreate("/tokens"), {
      ...options,
      method: this.http.methodForCreate(),
      body: data,
    });
  }

  async delete(id: string, options?: RequestOptions): Promise<void> {
    await this.http.request<void>(this.http.pathForDelete("/tokens", id), {
      ...options,
      method: this.http.methodForDelete(),
    });
  }
}
