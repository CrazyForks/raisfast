import { HttpClient } from "./client";
import type { BatchRequest, BatchResponse, PaginatedData, RequestOptions, UpdateUserRequest, User, UserRole, UserResponse } from "./types";

export class Users {
  private readonly http: HttpClient;

  constructor(http: HttpClient) {
    this.http = http;
  }

  async getMe(options?: RequestOptions): Promise<User> {
    return this.http.get<User>("/users/me", options);
  }

  async updateMe(
    data: { username?: string; bio?: string; website?: string; avatar?: string; social_links?: Record<string, string>; metadata?: unknown },
    options?: RequestOptions,
  ): Promise<User> {
    return this.http.put<User>("/users/me", data, options);
  }

  async changePassword(
    data: { old_password: string; new_password: string },
    options?: RequestOptions,
  ): Promise<void> {
    await this.http.put("/users/me/password", data, options);
  }

  async getUser(id: string, options?: RequestOptions): Promise<User> {
    return this.http.get<User>(`/users/${id}`, options);
  }

  async listUsers(
    page = 1,
    pageSize = 25,
    options?: RequestOptions,
  ): Promise<PaginatedData<User>> {
    return this.http.get<PaginatedData<User>>("/users", {
      ...options,
      query: { page: String(page), page_size: String(pageSize) },
    });
  }

  async updateUserRole(
    id: string,
    role: UserRole,
    options?: RequestOptions,
  ): Promise<User> {
    return this.http.put<User>(`/users/${id}/role`, { role }, options);
  }

  async adminList(
    page = 1,
    pageSize = 25,
    options?: RequestOptions,
  ): Promise<PaginatedData<UserResponse>> {
    return this.http.get<PaginatedData<UserResponse>>("/admin/users", {
      ...options,
      query: { page: String(page), page_size: String(pageSize) },
    });
  }

  async adminGet(id: string, options?: RequestOptions): Promise<UserResponse> {
    return this.http.get<UserResponse>(`/admin/users/${id}`, options);
  }

  async adminUpdate(
    id: string,
    data: UpdateUserRequest,
    options?: RequestOptions,
  ): Promise<UserResponse> {
    return this.http.put<UserResponse>(`/admin/users/${id}`, data, options);
  }

  async adminDelete(id: string, options?: RequestOptions): Promise<void> {
    await this.http.del(`/admin/users/${id}`, options);
  }

  async adminBatch(
    data: BatchRequest & { role?: UserRole },
    options?: RequestOptions,
  ): Promise<BatchResponse> {
    return this.http.post<BatchResponse>("/admin/users/batch", data, options);
  }
}
