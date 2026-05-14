import { HttpClient } from "../client";
import type {
  PaginatedData,
  RequestOptions,
  StepLog,
  Workflow,
  WorkflowInstance,
} from "../types";

export class AdminWorkflows {
  private readonly http: HttpClient;

  constructor(http: HttpClient) {
    this.http = http;
  }

  async list(options?: RequestOptions): Promise<Workflow[]> {
    return this.http.get<Workflow[]>("/admin/workflows", options);
  }

  async create(
    data: { name: string; steps: unknown[] },
    options?: RequestOptions,
  ): Promise<Workflow> {
    return this.http.request<Workflow>(this.http.pathForCreate("/admin/workflows"), {
      ...options,
      method: this.http.methodForCreate(),
      body: data,
    });
  }

  async get(id: string, options?: RequestOptions): Promise<Workflow> {
    return this.http.get<Workflow>(`/admin/workflows/${id}`, options);
  }

  async delete(id: string, options?: RequestOptions): Promise<void> {
    await this.http.request<void>(this.http.pathForDelete("/admin/workflows", id), {
      ...options,
      method: this.http.methodForDelete(),
    });
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
