import { HttpClient, toQueryString } from "../client";
import type {
  PaginatedData,
  PostListQuery,
  PostResponse,
  PostStatus,
  RequestOptions,
} from "../types";

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
      query: toQueryString(query as unknown as Record<string, string | number | undefined>),
    });
  }

  async get(slug: string, options?: RequestOptions): Promise<PostResponse> {
    return this.http.get<PostResponse>(`/posts/${slug}`, options);
  }

  async create(
    body: CreatePostBody,
    options?: RequestOptions,
  ): Promise<PostResponse> {
    return this.http.request<PostResponse>(this.http.pathForCreate("/posts"), {
      ...options,
      method: this.http.methodForCreate(),
      body,
    });
  }

  async update(
    slug: string,
    body: UpdatePostBody,
    options?: RequestOptions,
  ): Promise<PostResponse> {
    return this.http.request<PostResponse>(this.http.pathForUpdate("/posts", slug), {
      ...options,
      method: this.http.methodForUpdate(),
      body,
    });
  }

  async delete(slug: string, options?: RequestOptions): Promise<void> {
    await this.http.request<void>(this.http.pathForDelete("/posts", slug), {
      ...options,
      method: this.http.methodForDelete(),
    });
  }
}
