import { HttpClient } from "./client";
import type {
  BatchRequest,
  BatchResponse,
  RequestOptions,
  ReusableBlock,
} from "./types";

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

export class ReusableBlocks {
  private readonly http: HttpClient;

  constructor(http: HttpClient) {
    this.http = http;
  }

  async list(options?: RequestOptions): Promise<ReusableBlock[]> {
    return this.http.get<ReusableBlock[]>("/admin/reusable-blocks", options);
  }

  async get(id: string, options?: RequestOptions): Promise<ReusableBlock> {
    return this.http.get<ReusableBlock>(`/admin/reusable-blocks/${id}`, options);
  }

  async create(
    body: CreateReusableBlockBody,
    options?: RequestOptions,
  ): Promise<ReusableBlock> {
    return this.http.post<ReusableBlock>("/admin/reusable-blocks", body, options);
  }

  async update(
    id: string,
    body: UpdateReusableBlockBody,
    options?: RequestOptions,
  ): Promise<ReusableBlock> {
    return this.http.put<ReusableBlock>(`/admin/reusable-blocks/${id}`, body, options);
  }

  async delete(id: string, options?: RequestOptions): Promise<void> {
    await this.http.del(`/admin/reusable-blocks/${id}`, options);
  }

  async batch(
    data: BatchRequest,
    options?: RequestOptions,
  ): Promise<BatchResponse> {
    return this.http.post<BatchResponse>("/admin/reusable-blocks/batch", data, options);
  }
}
