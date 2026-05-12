import { HttpClient } from "../client";
import type { BatchResponse, PaginatedData, PluginInfoResponse, RequestOptions } from "../types";

export class AdminPlugins {
  private readonly http: HttpClient;

  constructor(http: HttpClient) {
    this.http = http;
  }

  async list(
    page = 1,
    pageSize = 25,
    options?: RequestOptions,
  ): Promise<PaginatedData<PluginInfoResponse>> {
    return this.http.get<PaginatedData<PluginInfoResponse>>("/admin/plugins", {
      ...options,
      query: { page: String(page), page_size: String(pageSize) },
    });
  }

  async get(id: string, options?: RequestOptions): Promise<PluginInfoResponse> {
    return this.http.get<PluginInfoResponse>(`/admin/plugins/${id}`, options);
  }

  async enable(id: string, options?: RequestOptions): Promise<void> {
    await this.http.post(`/admin/plugins/${id}/enable`, {}, options);
  }

  async disable(id: string, options?: RequestOptions): Promise<void> {
    await this.http.post(`/admin/plugins/${id}/disable`, {}, options);
  }

  async reload(id: string, options?: RequestOptions): Promise<void> {
    await this.http.post(`/admin/plugins/${id}/reload`, {}, options);
  }

  async unload(id: string, options?: RequestOptions): Promise<void> {
    await this.http.del(`/admin/plugins/${id}`, options);
  }

  async batch(
    data: { action: string; ids: string[] },
    options?: RequestOptions,
  ): Promise<BatchResponse> {
    return this.http.post("/admin/plugins/batch", data, options);
  }
}
