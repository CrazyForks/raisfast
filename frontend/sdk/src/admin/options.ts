import { HttpClient } from "../client";
import type { OptionGroup, RequestOptions } from "../types";

export class AdminOptions {
  private readonly http: HttpClient;

  constructor(http: HttpClient) {
    this.http = http;
  }

  async list(options?: RequestOptions): Promise<OptionGroup[]> {
    return this.http.get<OptionGroup[]>("/admin/options", options);
  }

  async get(key: string, options?: RequestOptions): Promise<unknown> {
    return this.http.get<unknown>(`/admin/options/${key}`, options);
  }

  async set(
    key: string,
    value: string,
    options?: RequestOptions,
  ): Promise<void> {
    await this.http.request<void>(this.http.pathForUpdate("/admin/options", key), {
      ...options,
      method: this.http.methodForUpdate(),
      body: { value },
    });
  }

  async delete(key: string, options?: RequestOptions): Promise<void> {
    await this.http.request<void>(this.http.pathForDelete("/admin/options", key), {
      ...options,
      method: this.http.methodForDelete(),
    });
  }

  async batchUpdate(
    data: Record<string, unknown>,
    options?: RequestOptions,
  ): Promise<void> {
    await this.http.request<void>(this.http.pathForUpdate("/admin/options", ""), {
      ...options,
      method: this.http.methodForUpdate(),
      body: { options: data },
    });
  }

  async getPublic(
    options?: RequestOptions,
  ): Promise<Record<string, string>> {
    return this.http.get<Record<string, string>>("/options/public", options);
  }
}
