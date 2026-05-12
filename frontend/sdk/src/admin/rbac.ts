import { HttpClient } from "../client";
import type {
  BatchResponse,
  Permission,
  RequestOptions,
  Role,
} from "../types";

export class AdminRBAC {
  private readonly http: HttpClient;

  constructor(http: HttpClient) {
    this.http = http;
  }

  async listRoles(options?: RequestOptions): Promise<Role[]> {
    return this.http.get<Role[]>("/admin/rbac/roles", options);
  }

  async createRole(
    data: { name: string; description?: string },
    options?: RequestOptions,
  ): Promise<Role> {
    return this.http.post<Role>("/admin/rbac/roles", data, options);
  }

  async updateRole(
    id: string,
    data: Partial<Role>,
    options?: RequestOptions,
  ): Promise<Role> {
    return this.http.put<Role>(`/admin/rbac/roles/${id}`, data, options);
  }

  async deleteRole(id: string, options?: RequestOptions): Promise<void> {
    await this.http.del(`/admin/rbac/roles/${id}`, options);
  }

  async batchRoles(
    data: { action: string; ids: string[] },
    options?: RequestOptions,
  ): Promise<BatchResponse> {
    return this.http.post("/admin/rbac/roles/batch", data, options);
  }

  async getPermissions(
    roleId: string,
    options?: RequestOptions,
  ): Promise<Permission[]> {
    return this.http.get<Permission[]>(
      `/admin/rbac/roles/${roleId}/permissions`,
      options,
    );
  }

  async setPermissions(
    roleId: string,
    permissions: Permission[],
    options?: RequestOptions,
  ): Promise<void> {
    await this.http.put(
      `/admin/rbac/roles/${roleId}/permissions`,
      permissions,
      options,
    );
  }
}
