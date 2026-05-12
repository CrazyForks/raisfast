import { HttpClient, toQueryString } from "../client";
import type { RequestOptions, TrendPoint } from "../types";

export class AdminStats {
  private readonly http: HttpClient;

  constructor(http: HttpClient) {
    this.http = http;
  }

  async overview(options?: RequestOptions): Promise<{
    posts: number;
    pages: number;
    comments: number;
    categories: number;
    tags: number;
    media: number;
    users: number;
  }> {
    return this.http.get("/admin/stats", options);
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
    return this.http.get<TrendPoint[]>("/admin/stats/trends", {
      ...options,
      query: toQueryString({ table, days }),
    });
  }
}
