import { HttpClient } from "../client";
import type { HealthStatus, RequestOptions } from "../types";

export class Health {
  private readonly http: HttpClient;

  constructor(http: HttpClient) {
    this.http = http;
  }

  async check(options?: RequestOptions): Promise<HealthStatus> {
    return this.http.get<HealthStatus>("/health", options);
  }

  async liveness(options?: RequestOptions): Promise<HealthStatus> {
    const base = this.http.baseUrl.replace(/\/api\/v\d+\/?$/, "");
    const res = await fetch(`${base}/healthz`);
    return res.json();
  }

  async readiness(options?: RequestOptions): Promise<HealthStatus> {
    const base = this.http.baseUrl.replace(/\/api\/v\d+\/?$/, "");
    const res = await fetch(`${base}/readyz`);
    return res.json();
  }
}
