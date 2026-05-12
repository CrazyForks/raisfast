import { HttpClient, toQueryString } from "../client";
import type {
  AdminCommentRow,
  BatchRequest,
  BatchResponse,
  CommentStatus,
  PaginatedData,
  RequestOptions,
} from "../types";

export class AdminComments {
  private readonly http: HttpClient;

  constructor(http: HttpClient) {
    this.http = http;
  }

  async list(
    query?: { page?: number; page_size?: number; status?: CommentStatus },
    options?: RequestOptions,
  ): Promise<PaginatedData<AdminCommentRow>> {
    return this.http.get<PaginatedData<AdminCommentRow>>("/admin/comments", {
      ...options,
      query: toQueryString(query as Record<string, string | number | undefined>),
    });
  }

  async updateStatus(
    id: string,
    status: CommentStatus,
    options?: RequestOptions,
  ): Promise<void> {
    await this.http.put(`/admin/comments/${id}/status`, { status }, options);
  }

  async delete(id: string, options?: RequestOptions): Promise<void> {
    await this.http.del(`/admin/comments/${id}`, options);
  }

  async batch(
    data: BatchRequest,
    options?: RequestOptions,
  ): Promise<BatchResponse> {
    return this.http.post<BatchResponse>(
      "/admin/comments/batch",
      data,
      options,
    );
  }
}
