import { HttpClient } from "../client";
import type {
  BatchRequest,
  BatchResponse,
  RequestOptions,
  ReusableBlock,
} from "../types";

export interface CreateReusableBlockBody {
  name: string;
  block_type: string;
  content: string;
  description?: string;
}

export interface UpdateReusableBlockBody {
  name?: string;
  block_type?: string;
  content?: string;
  description?: string;
}

export class AdminReusableBlocks {
  private readonly http: HttpClient;

  constructor(http: HttpClient) {
    this.http = http;
  }

  async list(options?: RequestOptions): Promise<ReusableBlock[]> {
    return this.http.get<ReusableBlock[]>("/admin/reusable-blocks", options);
  }

  async get(id: string, options?: RequestOptions): Promise<ReusableBlock> {
    return this.http.get<ReusableBlock>(
      `/admin/reusable-blocks/${id}`,
      options,
    );
  }

  async create(
    body: CreateReusableBlockBody,
    options?: RequestOptions,
  ): Promise<ReusableBlock> {
    return this.http.request<ReusableBlock>(this.http.pathForCreate("/admin/reusable-blocks"), {
      ...options,
      method: this.http.methodForCreate(),
      body,
    });
  }

  async update(
    id: string,
    body: UpdateReusableBlockBody,
    options?: RequestOptions,
  ): Promise<ReusableBlock> {
    return this.http.request<ReusableBlock>(this.http.pathForUpdate("/admin/reusable-blocks", id), {
      ...options,
      method: this.http.methodForUpdate(),
      body,
    });
  }

  async delete(id: string, options?: RequestOptions): Promise<void> {
    await this.http.request<void>(this.http.pathForDelete("/admin/reusable-blocks", id), {
      ...options,
      method: this.http.methodForDelete(),
    });
  }

  async batch(
    data: BatchRequest,
    options?: RequestOptions,
  ): Promise<BatchResponse> {
    return this.http.post<BatchResponse>(
      "/admin/reusable-blocks/batch",
      data,
      options,
    );
  }
}
