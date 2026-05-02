import { HttpClient } from "./client";
import type { HealthStatus, RequestOptions } from "./types";

export class Health {
  private readonly http: HttpClient;

  constructor(http: HttpClient) {
    this.http = http;
  }

  async check(options?: RequestOptions): Promise<HealthStatus> {
    return this.http.get<HealthStatus>("/health", options);
  }

  async liveness(options?: RequestOptions): Promise<HealthStatus> {
    return this.http.get<HealthStatus>("/healthz", options);
  }

  async readiness(options?: RequestOptions): Promise<HealthStatus> {
    return this.http.get<HealthStatus>("/readyz", options);
  }
}
