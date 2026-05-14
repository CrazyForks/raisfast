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
    return this.http.request<Category>(this.http.pathForCreate("/admin/categories"), {
      ...options,
      method: this.http.methodForCreate(),
      body,
    });
  }

  async update(
    id: string,
    body: UpdateCategoryRequest,
    options?: RequestOptions,
  ): Promise<Category> {
    return this.http.request<Category>(this.http.pathForUpdate("/admin/categories", id), {
      ...options,
      method: this.http.methodForUpdate(),
      body,
    });
  }

  async delete(id: string, options?: RequestOptions): Promise<void> {
    await this.http.request<void>(this.http.pathForDelete("/admin/categories", id), {
      ...options,
      method: this.http.methodForDelete(),
    });
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
