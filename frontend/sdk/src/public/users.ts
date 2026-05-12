import { HttpClient } from "../client";
import type { PaginatedData, RequestOptions, User, UserRole } from "../types";

export class Users {
  private readonly http: HttpClient;

  constructor(http: HttpClient) {
    this.http = http;
  }

  async getMe(options?: RequestOptions): Promise<User> {
    return this.http.get<User>("/users/me", options);
  }

  async updateMe(
    data: {
      username?: string;
      bio?: string;
      website?: string;
      avatar?: string;
      social_links?: Record<string, string>;
      metadata?: unknown;
    },
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
}
