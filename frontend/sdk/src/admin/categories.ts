import { HttpClient } from "../client";
import type {
  BatchRequest,
  BatchResponse,
  Category,
  CreateCategoryRequest,
  PaginatedData,
  RequestOptions,
  UpdateCategoryRequest,
} from "../types";

export class AdminCategories {
  private readonly http: HttpClient;

  constructor(http: HttpClient) {
    this.http = http;
  }

  async list(
    page = 1,
    pageSize = 25,
    options?: RequestOptions,
  ): Promise<PaginatedData<Category>> {
    return this.http.get<PaginatedData<Category>>("/admin/categories", {
      ...options,
      query: { page: String(page), page_size: String(pageSize) },
    });
  }

  async create(
    body: CreateCategoryRequest,
    options?: RequestOptions,
  ): Promise<Category> {
    return this.http.post<Category>("/admin/categories", body, options);
  }

  async update(
    id: string,
    body: UpdateCategoryRequest,
    options?: RequestOptions,
  ): Promise<Category> {
    return this.http.put<Category>(`/admin/categories/${id}`, body, options);
  }

  async delete(id: string, options?: RequestOptions): Promise<void> {
    await this.http.del(`/admin/categories/${id}`, options);
  }

  async batch(
    data: BatchRequest,
    options?: RequestOptions,
  ): Promise<BatchResponse> {
    return this.http.post<BatchResponse>(
      "/admin/categories/batch",
      data,
      options,
    );
  }
}
