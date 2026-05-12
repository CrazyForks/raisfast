import { HttpClient } from "./client";
import type {
  AdminStats as AdminStatsType,
  ApiToken,
  AuditLog,
  ContentTypeSchema,
  CronJob,
  CronLog,
  PaginatedData,
  Permission,
  PluginInfo,
  RequestOptions,
  Role,
  RouteInfo,
  StepLog,
  Tenant,
  TrendPoint,
  Webhook,
  Workflow,
  WorkflowInstance,
} from "./types";

class AdminPlugins {
  constructor(private readonly http: HttpClient) {}

  async list(options?: RequestOptions): Promise<PluginInfo[]> {
    return this.http.get<PluginInfo[]>("/admin/plugins", options);
  }

  async get(id: string, options?: RequestOptions): Promise<PluginInfo> {
    return this.http.get<PluginInfo>(`/admin/plugins/${id}`, options);
  }

  async enable(id: string, options?: RequestOptions): Promise<void> {
    await this.http.post(`/admin/plugins/${id}/enable`, {}, options);
  }

  async disable(id: string, options?: RequestOptions): Promise<void> {
    await this.http.post(`/admin/plugins/${id}/disable`, {}, options);
  }

  async reload(id: string, options?: RequestOptions): Promise<void> {
    await this.http.post(`/admin/plugins/${id}/reload`, {}, options);
  }

  async unload(id: string, options?: RequestOptions): Promise<void> {
    await this.http.del(`/admin/plugins/${id}`, options);
  }

  async batch(
    data: { action: string; ids: string[] },
    options?: RequestOptions,
  ): Promise<{ action: string; affected: number }> {
    return this.http.post("/admin/plugins/batch", data, options);
  }
}

class AdminContentTypes {
  constructor(private readonly http: HttpClient) {}

  async list(options?: RequestOptions): Promise<ContentTypeSchema[]> {
    return this.http.get<ContentTypeSchema[]>("/admin/content-types", options);
  }

  async get(name: string, options?: RequestOptions): Promise<ContentTypeSchema> {
    return this.http.get<ContentTypeSchema>(`/admin/content-types/${name}`, options);
  }

  async create(schema: ContentTypeSchema, options?: RequestOptions): Promise<ContentTypeSchema> {
    return this.http.post<ContentTypeSchema>("/admin/content-types", schema, options);
  }

  async update(
    name: string,
    schema: Partial<ContentTypeSchema>,
    options?: RequestOptions,
  ): Promise<ContentTypeSchema> {
    return this.http.put<ContentTypeSchema>(`/admin/content-types/${name}`, schema, options);
  }

  async delete(name: string, options?: RequestOptions): Promise<void> {
    await this.http.del(`/admin/content-types/${name}`, options);
  }
}

class AdminTenants {
  constructor(private readonly http: HttpClient) {}

  async list(
    page = 1,
    pageSize = 25,
    options?: RequestOptions,
  ): Promise<PaginatedData<Tenant>> {
    return this.http.get<PaginatedData<Tenant>>("/admin/tenants", {
      ...options,
      query: { page: String(page), page_size: String(pageSize) },
    });
  }

  async get(id: string, options?: RequestOptions): Promise<Tenant> {
    return this.http.get<Tenant>(`/admin/tenants/${id}`, options);
  }

  async create(
    data: { name: string; slug: string },
    options?: RequestOptions,
  ): Promise<Tenant> {
    return this.http.post<Tenant>("/admin/tenants", data, options);
  }

  async update(
    id: string,
    data: Partial<Tenant>,
    options?: RequestOptions,
  ): Promise<Tenant> {
    return this.http.put<Tenant>(`/admin/tenants/${id}`, data, options);
  }

  async delete(id: string, options?: RequestOptions): Promise<void> {
    await this.http.del(`/admin/tenants/${id}`, options);
  }

  async batch(
    data: { action: string; ids: string[] },
    options?: RequestOptions,
  ): Promise<{ action: string; affected: number }> {
    return this.http.post("/admin/tenants/batch", data, options);
  }
}

class AdminRBAC {
  constructor(private readonly http: HttpClient) {}

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
  ): Promise<{ action: string; affected: number }> {
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

class AdminOptions {
  constructor(private readonly http: HttpClient) {}

  async list(options?: RequestOptions): Promise<Record<string, string>> {
    return this.http.get<Record<string, string>>("/admin/options", options);
  }

  async get(key: string, options?: RequestOptions): Promise<string> {
    return this.http.get<string>(`/admin/options/${key}`, options);
  }

  async set(
    key: string,
    value: string,
    options?: RequestOptions,
  ): Promise<void> {
    await this.http.put(`/admin/options/${key}`, { value }, options);
  }

  async delete(key: string, options?: RequestOptions): Promise<void> {
    await this.http.del(`/admin/options/${key}`, options);
  }

  async batchUpdate(
    data: Record<string, unknown>,
    options?: RequestOptions,
  ): Promise<void> {
    await this.http.put("/admin/options", { options: data }, options);
  }

  async getPublic(
    options?: RequestOptions,
  ): Promise<Record<string, string>> {
    return this.http.get<Record<string, string>>("/options/public", options);
  }
}

class AdminWebhooks {
  constructor(private readonly http: HttpClient) {}

  async list(options?: RequestOptions): Promise<Webhook[]> {
    return this.http.get<Webhook[]>("/admin/webhooks", options);
  }

  async create(
    data: { url: string; events: string[]; secret?: string },
    options?: RequestOptions,
  ): Promise<Webhook> {
    return this.http.post<Webhook>("/admin/webhooks", data, options);
  }

  async get(id: string, options?: RequestOptions): Promise<Webhook> {
    return this.http.get<Webhook>(`/admin/webhooks/${id}`, options);
  }

  async update(
    id: string,
    data: Partial<Webhook>,
    options?: RequestOptions,
  ): Promise<Webhook> {
    return this.http.put<Webhook>(`/admin/webhooks/${id}`, data, options);
  }

  async delete(id: string, options?: RequestOptions): Promise<void> {
    await this.http.del(`/admin/webhooks/${id}`, options);
  }
}

class AdminAudit {
  constructor(private readonly http: HttpClient) {}

  async list(
    page = 1,
    pageSize = 25,
    options?: RequestOptions,
  ): Promise<PaginatedData<AuditLog>> {
    return this.http.get<PaginatedData<AuditLog>>("/admin/audit", {
      ...options,
      query: { page: String(page), page_size: String(pageSize) },
    });
  }

  async get(id: string, options?: RequestOptions): Promise<AuditLog> {
    return this.http.get<AuditLog>(`/admin/audit/${id}`, options);
  }
}

class AdminCrons {
  constructor(private readonly http: HttpClient) {}

  async list(options?: RequestOptions): Promise<CronJob[]> {
    return this.http.get<CronJob[]>("/admin/crons", options);
  }

  async create(
    data: { name: string; schedule: string; handler: string },
    options?: RequestOptions,
  ): Promise<CronJob> {
    return this.http.post<CronJob>("/admin/crons", data, options);
  }

  async get(id: string, options?: RequestOptions): Promise<CronJob> {
    return this.http.get<CronJob>(`/admin/crons/${id}`, options);
  }

  async update(
    id: string,
    data: Partial<CronJob>,
    options?: RequestOptions,
  ): Promise<CronJob> {
    return this.http.put<CronJob>(`/admin/crons/${id}`, data, options);
  }

  async delete(id: string, options?: RequestOptions): Promise<void> {
    await this.http.del(`/admin/crons/${id}`, options);
  }

  async toggle(id: string, enabled: boolean, options?: RequestOptions): Promise<CronJob> {
    return this.http.post<CronJob>(`/admin/crons/${id}/toggle`, { enabled }, options);
  }

  async listLogs(
    params?: { schedule_id?: string; limit?: number },
    options?: RequestOptions,
  ): Promise<CronLog[]> {
    return this.http.get<CronLog[]>("/admin/crons/logs", {
      ...options,
      query: params as unknown as Record<string, string>,
    });
  }

  async cleanupLogs(options?: RequestOptions): Promise<void> {
    await this.http.post("/admin/crons/logs/cleanup", {}, options);
  }

  async batch(
    data: { action: string; ids: string[] },
    options?: RequestOptions,
  ): Promise<{ action: string; affected: number }> {
    return this.http.post("/admin/crons/batch", data, options);
  }
}

class AdminTokens {
  constructor(private readonly http: HttpClient) {}

  async list(options?: RequestOptions): Promise<ApiToken[]> {
    return this.http.get<ApiToken[]>("/tokens", options);
  }

  async create(
    data: { name: string; scopes: string[]; expires_at?: string },
    options?: RequestOptions,
  ): Promise<ApiToken & { token: string }> {
    return this.http.post<ApiToken & { token: string }>("/tokens", data, options);
  }

  async delete(id: string, options?: RequestOptions): Promise<void> {
    await this.http.del(`/tokens/${id}`, options);
  }
}

class AdminWorkflows {
  constructor(private readonly http: HttpClient) {}

  async list(options?: RequestOptions): Promise<Workflow[]> {
    return this.http.get<Workflow[]>("/admin/workflows", options);
  }

  async create(
    data: { name: string; steps: unknown[] },
    options?: RequestOptions,
  ): Promise<Workflow> {
    return this.http.post<Workflow>("/admin/workflows", data, options);
  }

  async get(id: string, options?: RequestOptions): Promise<Workflow> {
    return this.http.get<Workflow>(`/admin/workflows/${id}`, options);
  }

  async delete(id: string, options?: RequestOptions): Promise<void> {
    await this.http.del(`/admin/workflows/${id}`, options);
  }

  async start(
    id: string,
    payload?: Record<string, unknown>,
    options?: RequestOptions,
  ): Promise<WorkflowInstance> {
    return this.http.post<WorkflowInstance>(
      `/admin/workflows/${id}/start`,
      payload ?? {},
      options,
    );
  }

  async listInstances(
    page = 1,
    pageSize = 25,
    options?: RequestOptions,
  ): Promise<PaginatedData<WorkflowInstance>> {
    return this.http.get<PaginatedData<WorkflowInstance>>(
      "/admin/workflows/instances",
      { ...options, query: { page: String(page), page_size: String(pageSize) } },
    );
  }

  async getInstance(
    id: string,
    options?: RequestOptions,
  ): Promise<WorkflowInstance> {
    return this.http.get<WorkflowInstance>(
      `/admin/workflows/instances/${id}`,
      options,
    );
  }

  async executeStep(
    instanceId: string,
    data: { step: string; action: string; data?: Record<string, unknown> },
    options?: RequestOptions,
  ): Promise<WorkflowInstance> {
    return this.http.post<WorkflowInstance>(
      `/admin/workflows/instances/${instanceId}/execute`,
      data,
      options,
    );
  }

  async cancelInstance(
    instanceId: string,
    options?: RequestOptions,
  ): Promise<void> {
    await this.http.post(
      `/admin/workflows/instances/${instanceId}/cancel`,
      {},
      options,
    );
  }

  async getStepLogs(
    instanceId: string,
    options?: RequestOptions,
  ): Promise<StepLog[]> {
    return this.http.get<StepLog[]>(
      `/admin/workflows/instances/${instanceId}/logs`,
      options,
    );
  }
}

class AdminStats {
  constructor(private readonly http: HttpClient) {}

  async overview(options?: RequestOptions): Promise<AdminStatsType> {
    return this.http.get<AdminStatsType>("/admin/stats", options);
  }

  async content(
    table: string,
    options?: RequestOptions,
  ): Promise<Record<string, number>> {
    return this.http.get<Record<string, number>>(
      `/admin/stats/content/${table}`,
      options,
    );
  }

  async trends(
    table: string,
    days = 30,
    options?: RequestOptions,
  ): Promise<TrendPoint[]> {
    return this.http.get<TrendPoint[]>(
      `/admin/stats/trends?table=${table}&days=${days}`,
      options,
    );
  }
}

export class Admin {
  readonly plugins: AdminPlugins;
  readonly contentTypes: AdminContentTypes;
  readonly tenants: AdminTenants;
  readonly rbac: AdminRBAC;
  readonly options: AdminOptions;
  readonly webhooks: AdminWebhooks;
  readonly audit: AdminAudit;
  readonly crons: AdminCrons;
  readonly tokens: AdminTokens;
  readonly workflows: AdminWorkflows;
  readonly stats: AdminStats;

  private readonly http: HttpClient;

  constructor(http: HttpClient) {
    this.http = http;
    this.plugins = new AdminPlugins(http);
    this.contentTypes = new AdminContentTypes(http);
    this.tenants = new AdminTenants(http);
    this.rbac = new AdminRBAC(http);
    this.options = new AdminOptions(http);
    this.webhooks = new AdminWebhooks(http);
    this.audit = new AdminAudit(http);
    this.crons = new AdminCrons(http);
    this.tokens = new AdminTokens(http);
    this.workflows = new AdminWorkflows(http);
    this.stats = new AdminStats(http);
  }

  async listRoutes(options?: RequestOptions): Promise<RouteInfo[]> {
    return this.http.get<RouteInfo[]>("/routes", options);
  }
}
