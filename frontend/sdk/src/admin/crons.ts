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
