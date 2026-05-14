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
    return this.http.request<Tenant>(this.http.pathForCreate("/admin/tenants"), {
      ...options,
      method: this.http.methodForCreate(),
      body: data,
    });
  }

  async update(
    id: string,
    data: Partial<Tenant>,
    options?: RequestOptions,
  ): Promise<Tenant> {
    return this.http.request<Tenant>(this.http.pathForUpdate("/admin/tenants", id), {
      ...options,
      method: this.http.methodForUpdate(),
      body: data,
    });
  }

  async delete(id: string, options?: RequestOptions): Promise<void> {
    await this.http.request<void>(this.http.pathForDelete("/admin/tenants", id), {
      ...options,
      method: this.http.methodForDelete(),
    });
  }

  async batch(
    data: { action: string; ids: string[] },
    options?: RequestOptions,
  ): Promise<BatchResponse> {
    return this.http.post("/admin/tenants/batch", data, options);
  }
}
