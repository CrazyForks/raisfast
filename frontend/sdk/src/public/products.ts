import { HttpClient } from "../client";
import type { PaginatedData, ProductResponse, RequestOptions } from "../types";

export class Products {
  private readonly http: HttpClient;

  constructor(http: HttpClient) {
    this.http = http;
  }

  async list(
    page = 1,
    pageSize = 25,
    options?: RequestOptions,
  ): Promise<PaginatedData<ProductResponse>> {
    return this.http.get<PaginatedData<ProductResponse>>("/products", {
      ...options,
      query: { page: String(page), page_size: String(pageSize) },
    });
  }

  async get(id: string, options?: RequestOptions): Promise<ProductResponse> {
    return this.http.get<ProductResponse>(`/products/${id}`, options);
  }
}
