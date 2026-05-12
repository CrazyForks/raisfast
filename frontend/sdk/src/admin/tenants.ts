import { HttpClient } from "../client";
import type {
  BatchResponse,
  PaginatedData,
  RequestOptions,
  Tenant,
} from "../types";

export class AdminTenants {
  private readonly http: HttpClient;

  constructor(http: HttpClient) {
    this.http = http;
  }

  async list(
    page = 1,
    pageSize = 25,
    options?: RequestOptions,
  ): Promise<PaginatedData<Tenant>> {
    return this.http.get<PaginatedData<Tenant>>("/admin/tenants", {
      ...options,
      query: { page: String(page), page_size: String(pageSize) },
    });
  }

  async get(id: string, options?: RequestOptions): Promise<Tenant> {
    return this.http.get<Tenant>(`/admin/tenants/${id}`, options);
  }

  async create(
    data: { name: string; slug: string },
    options?: RequestOptions,
  ): Promise<Tenant> {
    return this.http.post<Tenant>("/admin/tenants", data, options);
  }

  async update(
    id: string,
    data: Partial<Tenant>,
    options?: RequestOptions,
  ): Promise<Tenant> {
    return this.http.put<Tenant>(`/admin/tenants/${id}`, data, options);
  }

  async delete(id: string, options?: RequestOptions): Promise<void> {
    await this.http.del(`/admin/tenants/${id}`, options);
  }

  async batch(
    data: { action: string; ids: string[] },
    options?: RequestOptions,
  ): Promise<BatchResponse> {
    return this.http.post("/admin/tenants/batch", data, options);
  }
}
