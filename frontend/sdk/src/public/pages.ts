import { HttpClient } from "../client";
import type { Page, PaginatedData, RequestOptions, SitemapEntry } from "../types";

export class Pages {
  private readonly http: HttpClient;

  constructor(http: HttpClient) {
    this.http = http;
  }

  async list(
    page = 1,
    pageSize = 25,
    options?: RequestOptions,
  ): Promise<PaginatedData<Page>> {
    return this.http.get<PaginatedData<Page>>("/pages", {
      ...options,
      query: { page: String(page), page_size: String(pageSize) },
    });
  }

  async get(slug: string, options?: RequestOptions): Promise<Page> {
    return this.http.get<Page>(`/pages/${slug}`, options);
  }

  async sitemap(options?: RequestOptions): Promise<SitemapEntry[]> {
    return this.http.get<SitemapEntry[]>("/pages/sitemap", options);
  }
}
