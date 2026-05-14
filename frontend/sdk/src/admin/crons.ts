import { HttpClient, toQueryString } from "../client";
import type {
  BatchResponse,
  CronJob,
  CronLog,
  RequestOptions,
} from "../types";

export class AdminCrons {
  private readonly http: HttpClient;

  constructor(http: HttpClient) {
    this.http = http;
  }

  async list(options?: RequestOptions): Promise<CronJob[]> {
    return this.http.get<CronJob[]>("/admin/crons", options);
  }

  async create(
    data: { name: string; schedule: string; handler: string },
    options?: RequestOptions,
  ): Promise<CronJob> {
    return this.http.request<CronJob>(this.http.pathForCreate("/admin/crons"), {
      ...options,
      method: this.http.methodForCreate(),
      body: data,
    });
  }

  async get(id: string, options?: RequestOptions): Promise<CronJob> {
    return this.http.get<CronJob>(`/admin/crons/${id}`, options);
  }

  async update(
    id: string,
    data: Partial<CronJob>,
    options?: RequestOptions,
  ): Promise<CronJob> {
    return this.http.request<CronJob>(this.http.pathForUpdate("/admin/crons", id), {
      ...options,
      method: this.http.methodForUpdate(),
      body: data,
    });
  }

  async delete(id: string, options?: RequestOptions): Promise<void> {
    await this.http.request<void>(this.http.pathForDelete("/admin/crons", id), {
      ...options,
      method: this.http.methodForDelete(),
    });
  }

  async toggle(
    id: string,
    enabled: boolean,
    options?: RequestOptions,
  ): Promise<CronJob> {
    return this.http.post<CronJob>(`/admin/crons/${id}/toggle`, { enabled }, options);
  }

  async listLogs(
    params?: { schedule_id?: string; limit?: number },
    options?: RequestOptions,
  ): Promise<CronLog[]> {
    return this.http.get<CronLog[]>("/admin/crons/logs", {
      ...options,
      query: toQueryString(params),
    });
  }

  async cleanupLogs(options?: RequestOptions): Promise<void> {
    await this.http.post("/admin/crons/logs/cleanup", {}, options);
  }

  async batch(
    data: { action: string; ids: string[] },
    options?: RequestOptions,
  ): Promise<BatchResponse> {
    return this.http.post("/admin/crons/batch", data, options);
  }
}
