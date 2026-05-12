import { HttpClient } from "../client";
import type {
  BatchRequest,
  BatchResponse,
  CreateTagRequest,
  PaginatedData,
  RequestOptions,
  Tag,
  UpdateTagRequest,
} from "../types";

export class AdminTags {
  private readonly http: HttpClient;

  constructor(http: HttpClient) {
    this.http = http;
  }

  async list(
    page = 1,
    pageSize = 25,
    options?: RequestOptions,
  ): Promise<PaginatedData<Tag>> {
    return this.http.get<PaginatedData<Tag>>("/admin/tags", {
      ...options,
      query: { page: String(page), page_size: String(pageSize) },
    });
  }

  async create(
    body: CreateTagRequest,
    options?: RequestOptions,
  ): Promise<Tag> {
    return this.http.post<Tag>("/admin/tags", body, options);
  }

  async update(
    id: string,
    body: UpdateTagRequest,
    options?: RequestOptions,
  ): Promise<Tag> {
    return this.http.put<Tag>(`/admin/tags/${id}`, body, options);
  }

  async delete(id: string, options?: RequestOptions): Promise<void> {
    await this.http.del(`/admin/tags/${id}`, options);
  }

  async batch(
    data: BatchRequest,
    options?: RequestOptions,
  ): Promise<BatchResponse> {
    return this.http.post<BatchResponse>("/admin/tags/batch", data, options);
  }
}
