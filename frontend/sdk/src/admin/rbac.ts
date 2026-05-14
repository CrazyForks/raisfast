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
    return this.http.request<Role>(this.http.pathForCreate("/admin/rbac/roles"), {
      ...options,
      method: this.http.methodForCreate(),
      body: data,
    });
  }

  async updateRole(
    id: string,
    data: Partial<Role>,
    options?: RequestOptions,
  ): Promise<Role> {
    return this.http.request<Role>(this.http.pathForUpdate("/admin/rbac/roles", id), {
      ...options,
      method: this.http.methodForUpdate(),
      body: data,
    });
  }

  async deleteRole(id: string, options?: RequestOptions): Promise<void> {
    await this.http.request<void>(this.http.pathForDelete("/admin/rbac/roles", id), {
      ...options,
      method: this.http.methodForDelete(),
    });
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
    await this.http.request<void>(this.http.pathForUpdate("/admin/rbac/roles", `${roleId}/permissions`), {
      ...options,
      method: this.http.methodForUpdate(),
      body: permissions,
    });
  }
}
