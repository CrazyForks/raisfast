import { HttpClient } from "../client";
import type {
  BatchRequest,
  BatchResponse,
  MediaFile,
  MediaStats,
  MutateOptions,
  PaginatedData,
  RequestOptions,
} from "../types";

export class AdminMedia {
  private readonly http: HttpClient;

  constructor(http: HttpClient) {
    this.http = http;
  }

  async upload(
    file: File,
    options?: MutateOptions,
  ): Promise<MediaFile> {
    const formData = new FormData();
    formData.append("file", file);
    return this.http.request<MediaFile>("/admin/media/upload", {
      ...options,
      method: "POST",
      body: formData,
    });
  }

  async list(
    page = 1,
    pageSize = 25,
    options?: RequestOptions,
  ): Promise<PaginatedData<MediaFile>> {
    return this.http.get<PaginatedData<MediaFile>>("/admin/media", {
      ...options,
      query: { page: String(page), page_size: String(pageSize) },
    });
  }

  async delete(id: string, options?: RequestOptions): Promise<void> {
    await this.http.request<void>(this.http.pathForDelete("/admin/media", id), {
      ...options,
      method: this.http.methodForDelete(),
    });
  }

  async batch(
    data: BatchRequest,
    options?: RequestOptions,
  ): Promise<BatchResponse> {
    return this.http.post<BatchResponse>("/admin/media/batch", data, options);
  }
}
