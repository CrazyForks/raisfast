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
    return this.http.request<Tag>(this.http.pathForCreate("/admin/tags"), {
      ...options,
      method: this.http.methodForCreate(),
      body,
    });
  }

  async update(
    id: string,
    body: UpdateTagRequest,
    options?: RequestOptions,
  ): Promise<Tag> {
    return this.http.request<Tag>(this.http.pathForUpdate("/admin/tags", id), {
      ...options,
      method: this.http.methodForUpdate(),
      body,
    });
  }

  async delete(id: string, options?: RequestOptions): Promise<void> {
    await this.http.request<void>(this.http.pathForDelete("/admin/tags", id), {
      ...options,
      method: this.http.methodForDelete(),
    });
  }

  async batch(
    data: BatchRequest,
    options?: RequestOptions,
  ): Promise<BatchResponse> {
    return this.http.post<BatchResponse>("/admin/tags/batch", data, options);
  }
}
