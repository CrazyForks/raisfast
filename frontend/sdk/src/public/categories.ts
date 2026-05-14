import { HttpClient } from "../client";
import type { Category, PaginatedData, RequestOptions } from "../types";

export interface CreateCategoryBody {
  name: string;
  description?: string;
  parent_id?: string;
  sort_order?: number;
}

export interface UpdateCategoryBody {
  name?: string;
  description?: string;
  parent_id?: string;
  sort_order?: number;
}

export class Categories {
  private readonly http: HttpClient;

  constructor(http: HttpClient) {
    this.http = http;
  }

  async list(
    page = 1,
    pageSize = 25,
    options?: RequestOptions,
  ): Promise<PaginatedData<Category>> {
    return this.http.get<PaginatedData<Category>>("/categories", {
      ...options,
      query: { page: String(page), page_size: String(pageSize) },
    });
  }

  async get(
    id: string,
    options?: RequestOptions,
  ): Promise<Category> {
    return this.http.get<Category>(`/categories/${id}`, options);
  }

  async create(
    body: CreateCategoryBody,
    options?: RequestOptions,
  ): Promise<Category> {
    return this.http.request<Category>(this.http.pathForCreate("/categories"), {
      ...options,
      method: this.http.methodForCreate(),
      body,
    });
  }

  async update(
    id: string,
    body: UpdateCategoryBody,
    options?: RequestOptions,
  ): Promise<Category> {
    return this.http.request<Category>(this.http.pathForUpdate("/categories", id), {
      ...options,
      method: this.http.methodForUpdate(),
      body,
    });
  }

  async delete(id: string, options?: RequestOptions): Promise<void> {
    await this.http.request<void>(this.http.pathForDelete("/categories", id), {
      ...options,
      method: this.http.methodForDelete(),
    });
  }
}
