import { HttpClient } from "../client";
import type {
  BatchRequestWithRole,
  BatchResponse,
  PaginatedData,
  RequestOptions,
  UpdateUserRequest,
  UserResponse,
} from "../types";

export class AdminUsers {
  private readonly http: HttpClient;

  constructor(http: HttpClient) {
    this.http = http;
  }

  async list(
    page = 1,
    pageSize = 25,
    options?: RequestOptions,
  ): Promise<PaginatedData<UserResponse>> {
    return this.http.get<PaginatedData<UserResponse>>("/admin/users", {
      ...options,
      query: { page: String(page), page_size: String(pageSize) },
    });
  }

  async get(id: string, options?: RequestOptions): Promise<UserResponse> {
    return this.http.get<UserResponse>(`/admin/users/${id}`, options);
  }

  async update(
    id: string,
    data: UpdateUserRequest,
    options?: RequestOptions,
  ): Promise<UserResponse> {
    return this.http.put<UserResponse>(`/admin/users/${id}`, data, options);
  }

  async delete(id: string, options?: RequestOptions): Promise<void> {
    await this.http.del(`/admin/users/${id}`, options);
  }

  async batch(
    data: BatchRequestWithRole,
    options?: RequestOptions,
  ): Promise<BatchResponse> {
    return this.http.post<BatchResponse>("/admin/users/batch", data, options);
  }
}
