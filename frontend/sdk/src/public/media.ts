import { HttpClient } from "../client";
import type {
  MediaFile,
  MediaStats,
  MutateOptions,
  PaginatedData,
  RequestOptions,
} from "../types";

export class Media {
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
    return this.http.request<MediaFile>("/media/upload", {
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
    return this.http.get<PaginatedData<MediaFile>>("/media", {
      ...options,
      query: { page: String(page), page_size: String(pageSize) },
    });
  }

  async stats(options?: RequestOptions): Promise<MediaStats> {
    return this.http.get<MediaStats>("/media/stats", options);
  }

  async delete(id: string, options?: RequestOptions): Promise<void> {
    await this.http.del(`/media/${id}`, options);
  }

  getFileURL(
    record: Record<string, unknown>,
    field: string,
    options?: { thumb?: string },
  ): string {
    const base = this.http.baseUrl.replace("/api/v1", "");
    const path = record[field];
    if (typeof path !== "string") return "";
    if (path.startsWith("http")) return path;
    let url = `${base}/uploads/${path}`;
    if (options?.thumb) {
      url += `?thumb=${encodeURIComponent(options.thumb)}`;
    }
    return url;
  }
}
