import { HttpClient, toQueryString } from "../client";
import type {
  BatchRequest,
  BatchResponse,
  Page,
  PageStatus,
  PaginatedData,
  RequestOptions,
} from "../types";

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

export class AdminPages {
  private readonly http: HttpClient;

  constructor(http: HttpClient) {
    this.http = http;
  }

  async list(
    query?: { page?: number; page_size?: number; status?: PageStatus },
    options?: RequestOptions,
  ): Promise<PaginatedData<Page>> {
    return this.http.get<PaginatedData<Page>>("/admin/pages", {
      ...options,
      query: toQueryString(query as Record<string, string | number | undefined>),
    });
  }

  async get(id: string, options?: RequestOptions): Promise<Page> {
    return this.http.get<Page>(`/admin/pages/${id}`, options);
  }

  async create(body: CreatePageBody, options?: RequestOptions): Promise<Page> {
    return this.http.post<Page>("/admin/pages", body, options);
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

  async batch(
    data: BatchRequest,
    options?: RequestOptions,
  ): Promise<BatchResponse> {
    return this.http.post<BatchResponse>("/admin/pages/batch", data, options);
  }
}
