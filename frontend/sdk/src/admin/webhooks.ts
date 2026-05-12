import { HttpClient } from "../client";
import type { BatchResponse, PaginatedData, RequestOptions, Webhook } from "../types";

export class AdminWebhooks {
  private readonly http: HttpClient;

  constructor(http: HttpClient) {
    this.http = http;
  }

  async list(
    page = 1,
    pageSize = 25,
    options?: RequestOptions,
  ): Promise<PaginatedData<Webhook>> {
    return this.http.get<PaginatedData<Webhook>>("/admin/webhooks", {
      ...options,
      query: { page: String(page), page_size: String(pageSize) },
    });
  }

  async create(
    data: { url: string; events: string[]; description?: string; enabled?: boolean },
    options?: RequestOptions,
  ): Promise<Webhook> {
    return this.http.post<Webhook>("/admin/webhooks", data, options);
  }

  async get(id: string, options?: RequestOptions): Promise<Webhook> {
    return this.http.get<Webhook>(`/admin/webhooks/${id}`, options);
  }

  async update(
    id: string,
    data: Partial<Webhook>,
    options?: RequestOptions,
  ): Promise<Webhook> {
    return this.http.put<Webhook>(`/admin/webhooks/${id}`, data, options);
  }

  async delete(id: string, options?: RequestOptions): Promise<void> {
    await this.http.del(`/admin/webhooks/${id}`, options);
  }

  async batch(
    data: { action: string; ids: string[] },
    options?: RequestOptions,
  ): Promise<BatchResponse> {
    return this.http.post("/admin/webhooks/batch", data, options);
  }
}
