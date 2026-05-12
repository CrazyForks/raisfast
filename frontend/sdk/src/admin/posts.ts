import { HttpClient } from "../client";
import type {
  BatchRequest,
  BatchResponse,
  PaginatedData,
  PostResponse,
  PostStatus,
  RequestOptions,
} from "../types";
import type { CreatePostBody, UpdatePostBody } from "../public/posts";

export class AdminPosts {
  private readonly http: HttpClient;

  constructor(http: HttpClient) {
    this.http = http;
  }

  async list(
    query?: { page?: number; page_size?: number; status?: PostStatus },
    options?: RequestOptions,
  ): Promise<PaginatedData<PostResponse>> {
    return this.http.get<PaginatedData<PostResponse>>("/admin/posts", {
      ...options,
      query: query as Record<string, string>,
    });
  }

  async get(
    slug: string,
    options?: RequestOptions,
  ): Promise<PostResponse> {
    return this.http.get<PostResponse>(`/admin/posts/${slug}`, options);
  }

  async create(
    body: CreatePostBody,
    options?: RequestOptions,
  ): Promise<PostResponse> {
    return this.http.post<PostResponse>("/admin/posts", body, options);
  }

  async update(
    id: string,
    body: UpdatePostBody,
    options?: RequestOptions,
  ): Promise<PostResponse> {
    return this.http.put<PostResponse>(`/admin/posts/${id}`, body, options);
  }

  async delete(id: string, options?: RequestOptions): Promise<void> {
    await this.http.del(`/admin/posts/${id}`, options);
  }

  async batch(
    data: BatchRequest,
    options?: RequestOptions,
  ): Promise<BatchResponse> {
    return this.http.post<BatchResponse>("/admin/posts/batch", data, options);
  }
}
