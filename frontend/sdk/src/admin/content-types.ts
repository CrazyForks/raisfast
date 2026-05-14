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
    return this.http.request<ContentTypeSchema>(this.http.pathForCreate("/admin/content-types"), {
      ...options,
      method: this.http.methodForCreate(),
      body: schema,
    });
  }

  async update(
    name: string,
    schema: Partial<ContentTypeSchema>,
    options?: RequestOptions,
  ): Promise<ContentTypeSchema> {
    return this.http.request<ContentTypeSchema>(this.http.pathForUpdate("/admin/content-types", name), {
      ...options,
      method: this.http.methodForUpdate(),
      body: schema,
    });
  }

  async delete(name: string, options?: RequestOptions): Promise<void> {
    await this.http.request<void>(this.http.pathForDelete("/admin/content-types", name), {
      ...options,
      method: this.http.methodForDelete(),
    });
  }
}
