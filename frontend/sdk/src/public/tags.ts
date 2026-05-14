import { HttpClient } from "../client";
import type { PaginatedData, RequestOptions, Tag } from "../types";

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

  async get(
    id: string,
    options?: RequestOptions,
  ): Promise<Tag> {
    return this.http.get<Tag>(`/tags/${id}`, options);
  }

  async create(
    body: { name: string },
    options?: RequestOptions,
  ): Promise<Tag> {
    return this.http.request<Tag>(this.http.pathForCreate("/tags"), {
      ...options,
      method: this.http.methodForCreate(),
      body,
    });
  }

  async update(
    id: string,
    body: { name: string },
    options?: RequestOptions,
  ): Promise<Tag> {
    return this.http.request<Tag>(this.http.pathForUpdate("/tags", id), {
      ...options,
      method: this.http.methodForUpdate(),
      body,
    });
  }

  async delete(id: string, options?: RequestOptions): Promise<void> {
    await this.http.request<void>(this.http.pathForDelete("/tags", id), {
      ...options,
      method: this.http.methodForDelete(),
    });
  }
}
