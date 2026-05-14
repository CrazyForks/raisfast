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
    return this.http.request<Page>(this.http.pathForCreate("/admin/pages"), {
      ...options,
      method: this.http.methodForCreate(),
      body,
    });
  }

  async update(
    id: string,
    body: UpdatePageBody,
    options?: RequestOptions,
  ): Promise<Page> {
    return this.http.request<Page>(this.http.pathForUpdate("/admin/pages", id), {
      ...options,
      method: this.http.methodForUpdate(),
      body,
    });
  }

  async updateStatus(
    id: string,
    status: PageStatus,
    options?: RequestOptions,
  ): Promise<Page> {
    return this.http.request<Page>(this.http.pathForUpdate("/admin/pages/status", id), {
      ...options,
      method: this.http.methodForUpdate(),
      body: { status },
    });
  }

  async delete(id: string, options?: RequestOptions): Promise<void> {
    await this.http.request<void>(this.http.pathForDelete("/admin/pages", id), {
      ...options,
      method: this.http.methodForDelete(),
    });
  }

  async reorder(
    items: Array<{ id: string; sort_order: number }>,
    options?: RequestOptions,
  ): Promise<void> {
    await this.http.request<void>(this.http.pathForUpdate("/admin/pages", "reorder"), {
      ...options,
      method: this.http.methodForUpdate(),
      body: { items },
    });
  }

  async batch(
    data: BatchRequest,
    options?: RequestOptions,
  ): Promise<BatchResponse> {
    return this.http.post<BatchResponse>("/admin/pages/batch", data, options);
  }
}
