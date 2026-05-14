import { HttpClient } from "../client";
import type {
  CommentResponse,
  CommentStatus,
  PaginatedData,
  RequestOptions,
} from "../types";

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
  ): Promise<PaginatedData<CommentResponse>> {
    return this.http.get<PaginatedData<CommentResponse>>("/comments", {
      ...options,
      query: { page: String(page), page_size: String(pageSize) },
    });
  }

  async updateStatus(
    id: string,
    status: CommentStatus,
    options?: RequestOptions,
  ): Promise<void> {
    await this.http.request<void>(this.http.pathForUpdate("/comments/status", id), {
      ...options,
      method: this.http.methodForUpdate(),
      body: { status },
    });
  }

  async delete(id: string, options?: RequestOptions): Promise<void> {
    await this.http.request<void>(this.http.pathForDelete("/comments", id), {
      ...options,
      method: this.http.methodForDelete(),
    });
  }
}
