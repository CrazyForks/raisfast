import { HttpClient } from "./client";
import type {
  BatchRequest,
  BatchResponse,
  PaginatedData,
  PostListQuery,
  PostResponse,
  PostStatus,
  RequestOptions,
} from "./types";

export type { PostListQuery };

export interface CreatePostBody {
  title: string;
  content: string;
  excerpt?: string;
  cover_image?: string;
  status?: PostStatus;
  category_id?: string;
  tag_ids?: string[];
}

export interface UpdatePostBody {
  title?: string;
  content?: string;
  excerpt?: string;
  cover_image?: string;
  status?: PostStatus;
  category_id?: string;
  tag_ids?: string[];
}

export class Posts {
  private readonly http: HttpClient;

  constructor(http: HttpClient) {
    this.http = http;
  }

  async list(
    query?: PostListQuery,
    options?: RequestOptions,
  ): Promise<PaginatedData<PostResponse>> {
    return this.http.get<PaginatedData<PostResponse>>("/posts", {
      ...options,
      query: query as unknown as Record<string, string>,
    });
  }

  async get(slug: string, options?: RequestOptions): Promise<PostResponse> {
    return this.http.get<PostResponse>(`/posts/${slug}`, options);
  }

  async create(
    body: CreatePostBody,
    options?: RequestOptions,
  ): Promise<PostResponse> {
    return this.http.post<PostResponse>("/posts", body, options);
  }

  async update(
    slug: string,
    body: UpdatePostBody,
    options?: RequestOptions,
  ): Promise<PostResponse> {
    return this.http.put<PostResponse>(`/posts/${slug}`, body, options);
  }

  async delete(slug: string, options?: RequestOptions): Promise<void> {
    await this.http.del(`/posts/${slug}`, options);
  }

  async adminList(
    query?: { page?: number; page_size?: number; status?: PostStatus },
    options?: RequestOptions,
  ): Promise<PaginatedData<PostResponse>> {
    return this.http.get<PaginatedData<PostResponse>>("/admin/posts", {
      ...options,
      query: query as Record<string, string>,
    });
  }

  async adminGet(
    slug: string,
    options?: RequestOptions,
  ): Promise<PostResponse> {
    return this.http.get<PostResponse>(`/admin/posts/${slug}`, options);
  }

  async adminCreate(
    body: CreatePostBody,
    options?: RequestOptions,
  ): Promise<PostResponse> {
    return this.http.post<PostResponse>("/admin/posts", body, options);
  }

  async adminUpdate(
    id: string,
    body: UpdatePostBody,
    options?: RequestOptions,
  ): Promise<PostResponse> {
    return this.http.put<PostResponse>(`/admin/posts/${id}`, body, options);
  }

  async adminDelete(id: string, options?: RequestOptions): Promise<void> {
    await this.http.del(`/admin/posts/${id}`, options);
  }

  async adminBatch(
    data: BatchRequest,
    options?: RequestOptions,
  ): Promise<BatchResponse> {
    return this.http.post<BatchResponse>("/admin/posts/batch", data, options);
  }
}
