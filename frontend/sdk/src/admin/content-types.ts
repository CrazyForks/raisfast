import { HttpClient } from "../client";
import type { ContentTypeSchema, RequestOptions } from "../types";

export class AdminContentTypes {
  private readonly http: HttpClient;

  constructor(http: HttpClient) {
    this.http = http;
  }

  async list(options?: RequestOptions): Promise<ContentTypeSchema[]> {
    return this.http.get<ContentTypeSchema[]>("/admin/content-types", options);
  }

  async get(
    name: string,
    options?: RequestOptions,
  ): Promise<ContentTypeSchema> {
    return this.http.get<ContentTypeSchema>(
      `/admin/content-types/${name}`,
      options,
    );
  }

  async create(
    schema: ContentTypeSchema,
    options?: RequestOptions,
  ): Promise<ContentTypeSchema> {
    return this.http.post<ContentTypeSchema>(
      "/admin/content-types",
      schema,
      options,
    );
  }

  async update(
    name: string,
    schema: Partial<ContentTypeSchema>,
    options?: RequestOptions,
  ): Promise<ContentTypeSchema> {
    return this.http.put<ContentTypeSchema>(
      `/admin/content-types/${name}`,
      schema,
      options,
    );
  }

  async delete(name: string, options?: RequestOptions): Promise<void> {
    await this.http.del(`/admin/content-types/${name}`, options);
  }
}
