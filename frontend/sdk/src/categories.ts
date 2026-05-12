import { HttpClient } from "./client";
import type {
  BatchRequest,
  BatchResponse,
  Category,
  CreateCategoryRequest,
  PaginatedData,
  RequestOptions,
  UpdateCategoryRequest,
} from "./types";

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

  async adminList(
    page = 1,
    pageSize = 25,
    options?: RequestOptions,
  ): Promise<PaginatedData<Category>> {
    return this.http.get<PaginatedData<Category>>("/admin/categories", {
      ...options,
      query: { page: String(page), page_size: String(pageSize) },
    });
  }

  async adminCreate(
    body: CreateCategoryRequest,
    options?: RequestOptions,
  ): Promise<Category> {
    return this.http.post<Category>("/admin/categories", body, options);
  }

  async adminUpdate(
    id: string,
    body: UpdateCategoryRequest,
    options?: RequestOptions,
  ): Promise<Category> {
    return this.http.put<Category>(`/admin/categories/${id}`, body, options);
  }

  async adminDelete(id: string, options?: RequestOptions): Promise<void> {
    await this.http.del(`/admin/categories/${id}`, options);
  }

  async adminBatch(
    data: BatchRequest,
    options?: RequestOptions,
  ): Promise<BatchResponse> {
    return this.http.post<BatchResponse>("/admin/categories/batch", data, options);
  }
}
