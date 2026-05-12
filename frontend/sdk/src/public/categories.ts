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
    return this.http.post<Category>("/categories", body, options);
  }

  async update(
    id: string,
    body: UpdateCategoryBody,
    options?: RequestOptions,
  ): Promise<Category> {
    return this.http.put<Category>(`/categories/${id}`, body, options);
  }

  async delete(id: string, options?: RequestOptions): Promise<void> {
    await this.http.del(`/categories/${id}`, options);
  }
}
