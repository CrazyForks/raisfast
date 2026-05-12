import { HttpClient, toQueryString } from "../client";
import type { RequestOptions } from "../types";

export interface StatsOverview {
  total_posts: number;
  total_comments: number;
  total_users: number;
  total_media: number;
  total_categories: number;
  total_tags: number;
  posts_by_status: Record<string, number>;
  comments_by_status: Record<string, number>;
  content_by_type: Record<string, number>;
  recent_activity: {
    type: string;
    title?: string;
    slug?: string;
    content?: string;
    at: string;
  }[];
}

export interface TrendsData {
  table: string;
  days: number;
  data: { date: string; count: number }[];
}

export class AdminStats {
  private readonly http: HttpClient;

  constructor(http: HttpClient) {
    this.http = http;
  }

  async overview(options?: RequestOptions): Promise<StatsOverview> {
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
  ): Promise<TrendsData> {
    return this.http.get<TrendsData>("/admin/stats/trends", {
      ...options,
      query: toQueryString({ table, days }),
    });
  }
}
