import { HttpClient } from "../client";
import type { AuditLog, PaginatedData, RequestOptions } from "../types";

export class AdminAudit {
  private readonly http: HttpClient;

  constructor(http: HttpClient) {
    this.http = http;
  }

  async list(
    page = 1,
    pageSize = 25,
    options?: RequestOptions,
  ): Promise<PaginatedData<AuditLog>> {
    return this.http.get<PaginatedData<AuditLog>>("/admin/audit", {
      ...options,
      query: { page: String(page), page_size: String(pageSize) },
    });
  }

  async get(id: string, options?: RequestOptions): Promise<AuditLog> {
    return this.http.get<AuditLog>(`/admin/audit/${id}`, options);
  }
}
