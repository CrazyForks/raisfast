import { HttpClient } from "./client";
import type {
  BatchRequest,
  BatchResponse,
  Page,
  PageStatus,
  PaginatedData,
  RequestOptions,
  SitemapEntry,
} from "./types";

export interface CreatePageBody {
  title: string;
  slug?: string;
  content?: string;
  blocks?: string;
  meta_title?: string;
  meta_description?: string;
  og_image?: string;
  template?: string;
  parent_id?: string;
  sort_order?: number;
  status?: PageStatus;
  cover_image?: string;
}

export interface UpdatePageBody {
  title?: string;
  slug?: string;
  content?: string;
  blocks?: string;
  meta_title?: string;
  meta_description?: string;
  og_image?: string;
  template?: string;
  parent_id?: string | null;
  sort_order?: number;
  status?: PageStatus;
  cover_image?: string;
}

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

  async create(
    body: CreatePageBody,
    options?: RequestOptions,
  ): Promise<Page> {
    return this.http.post<Page>("/admin/pages", body, options);
  }

  async adminList(
    query?: { page?: number; page_size?: number; status?: PageStatus },
    options?: RequestOptions,
  ): Promise<PaginatedData<Page>> {
    return this.http.get<PaginatedData<Page>>("/admin/pages", {
      ...options,
      query: query as Record<string, string>,
    });
  }

  async adminGet(id: string, options?: RequestOptions): Promise<Page> {
    return this.http.get<Page>(`/admin/pages/${id}`, options);
  }

  async update(
    id: string,
    body: UpdatePageBody,
    options?: RequestOptions,
  ): Promise<Page> {
    return this.http.put<Page>(`/admin/pages/${id}`, body, options);
  }

  async updateStatus(
    id: string,
    status: PageStatus,
    options?: RequestOptions,
  ): Promise<Page> {
    return this.http.put<Page>(`/admin/pages/${id}/status`, { status }, options);
  }

  async delete(id: string, options?: RequestOptions): Promise<void> {
    await this.http.del(`/admin/pages/${id}`, options);
  }

  async reorder(
    items: Array<{ id: string; sort_order: number }>,
    options?: RequestOptions,
  ): Promise<void> {
    await this.http.put("/admin/pages/reorder", { items }, options);
  }

  async adminBatch(
    data: BatchRequest,
    options?: RequestOptions,
  ): Promise<BatchResponse> {
    return this.http.post<BatchResponse>("/admin/pages/batch", data, options);
  }
}
