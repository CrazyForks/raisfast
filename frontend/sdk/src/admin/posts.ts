import { HttpClient, toQueryString } from "../client";
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
      query: toQueryString(query as Record<string, string | number | undefined>),
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
    return this.http.request<PostResponse>(this.http.pathForCreate("/admin/posts"), {
      ...options,
      method: this.http.methodForCreate(),
      body,
    });
  }

  async update(
    id: string,
    body: UpdatePostBody,
    options?: RequestOptions,
  ): Promise<PostResponse> {
    return this.http.request<PostResponse>(this.http.pathForUpdate("/admin/posts", id), {
      ...options,
      method: this.http.methodForUpdate(),
      body,
    });
  }

  async delete(id: string, options?: RequestOptions): Promise<void> {
    await this.http.request<void>(this.http.pathForDelete("/admin/posts", id), {
      ...options,
      method: this.http.methodForDelete(),
    });
  }

  async batch(
    data: BatchRequest,
    options?: RequestOptions,
  ): Promise<BatchResponse> {
    return this.http.post<BatchResponse>("/admin/posts/batch", data, options);
  }
}
