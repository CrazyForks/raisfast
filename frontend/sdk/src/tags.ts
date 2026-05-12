import { HttpClient } from "./client";
import type {
  BatchRequest,
  BatchResponse,
  PaginatedData,
  RequestOptions,
  Tag,
} from "./types";

export class Tags {
  private readonly http: HttpClient;

  constructor(http: HttpClient) {
    this.http = http;
  }

  async list(
    page = 1,
    pageSize = 25,
    options?: RequestOptions,
  ): Promise<PaginatedData<Tag>> {
    return this.http.get<PaginatedData<Tag>>("/tags", {
      ...options,
      query: { page: String(page), page_size: String(pageSize) },
    });
  }

  async create(
    body: { name: string },
    options?: RequestOptions,
  ): Promise<Tag> {
    return this.http.post<Tag>("/tags", body, options);
  }

  async delete(id: string, options?: RequestOptions): Promise<void> {
    await this.http.del(`/tags/${id}`, options);
  }

  async update(
    id: string,
    body: { name: string },
    options?: RequestOptions,
  ): Promise<Tag> {
    return this.http.put<Tag>(`/tags/${id}`, body, options);
  }

  async adminList(
    page = 1,
    pageSize = 25,
    options?: RequestOptions,
  ): Promise<PaginatedData<Tag>> {
    return this.http.get<PaginatedData<Tag>>("/admin/tags", {
      ...options,
      query: { page: String(page), page_size: String(pageSize) },
    });
  }

  async adminCreate(
    body: { name: string },
    options?: RequestOptions,
  ): Promise<Tag> {
    return this.http.post<Tag>("/admin/tags", body, options);
  }

  async adminUpdate(
    id: string,
    body: { name: string },
    options?: RequestOptions,
  ): Promise<Tag> {
    return this.http.put<Tag>(`/admin/tags/${id}`, body, options);
  }

  async adminDelete(id: string, options?: RequestOptions): Promise<void> {
    await this.http.del(`/admin/tags/${id}`, options);
  }

  async adminBatch(
    data: BatchRequest,
    options?: RequestOptions,
  ): Promise<BatchResponse> {
    return this.http.post<BatchResponse>("/admin/tags/batch", data, options);
  }
}
