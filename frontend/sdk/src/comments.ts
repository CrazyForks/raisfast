import { HttpClient } from "./client";
import type {
  AdminCommentRow,
  BatchRequest,
  BatchResponse,
  CommentResponse,
  CommentStatus,
  PaginatedData,
  RequestOptions,
} from "./types";

export interface CreateCommentBody {
  content: string;
  parent_id?: string;
  nickname?: string;
  email?: string;
}

export class Comments {
  private readonly http: HttpClient;

  constructor(http: HttpClient) {
    this.http = http;
  }

  async list(
    postSlug: string,
    page = 1,
    pageSize = 25,
    options?: RequestOptions,
  ): Promise<PaginatedData<CommentResponse>> {
    return this.http.get<PaginatedData<CommentResponse>>(
      `/posts/${postSlug}/comments`,
      {
        ...options,
        query: { page: String(page), page_size: String(pageSize) },
      },
    );
  }

  async create(
    postSlug: string,
    body: CreateCommentBody,
    options?: RequestOptions,
  ): Promise<CommentResponse> {
    const isAuthenticated = this.http.authStore?.isAuthenticated;
    const endpoint = isAuthenticated
      ? `/posts/${postSlug}/comments/authed`
      : `/posts/${postSlug}/comments`;
    return this.http.post<CommentResponse>(endpoint, body, options);
  }

  async listAll(
    page = 1,
    pageSize = 25,
    options?: RequestOptions,
  ): Promise<PaginatedData<AdminCommentRow>> {
    return this.http.get<PaginatedData<AdminCommentRow>>("/comments", {
      ...options,
      query: { page: String(page), page_size: String(pageSize) },
    });
  }

  async updateStatus(
    id: string,
    status: CommentStatus,
    options?: RequestOptions,
  ): Promise<void> {
    await this.http.put(`/comments/${id}/status`, { status }, options);
  }

  async delete(id: string, options?: RequestOptions): Promise<void> {
    await this.http.del(`/comments/${id}`, options);
  }

  async adminList(
    query?: { page?: number; page_size?: number; status?: CommentStatus },
    options?: RequestOptions,
  ): Promise<PaginatedData<AdminCommentRow>> {
    return this.http.get<PaginatedData<AdminCommentRow>>("/admin/comments", {
      ...options,
      query: query as unknown as Record<string, string>,
    });
  }

  async adminUpdateStatus(
    id: string,
    status: CommentStatus,
    options?: RequestOptions,
  ): Promise<void> {
    await this.http.put(`/admin/comments/${id}/status`, { status }, options);
  }

  async adminDelete(id: string, options?: RequestOptions): Promise<void> {
    await this.http.del(`/admin/comments/${id}`, options);
  }

  async adminBatch(
    data: BatchRequest,
    options?: RequestOptions,
  ): Promise<BatchResponse> {
    return this.http.post<BatchResponse>("/admin/comments/batch", data, options);
  }
}
