import { HttpClient } from "../client";
import type { PaginatedData, ProductResponse, RequestOptions } from "../types";

export interface CreateProductBody {
  title: string;
  description?: string;
  cover_url?: string;
  category_id?: string;
  product_type?: string;
  fulfillment_type?: string;
  delivery_hook?: string;
  weight?: number;
  price: number;
  currency?: string;
  attributes?: string;
  sort_order?: number;
  slug?: string;
  content?: string;
  image_ids?: unknown;
  original_price?: number;
  specs?: unknown;
  unit?: string;
  min_purchase?: number;
  max_purchase?: number;
  virtual_sales?: number;
  meta_title?: string;
  meta_description?: string;
}

export interface UpdateProductBody {
  title?: string;
  description?: string;
  cover_url?: string;
  category_id?: string;
  product_type?: string;
  fulfillment_type?: string;
  delivery_hook?: string;
  weight?: number;
  price?: number;
  currency?: string;
  status?: string;
  attributes?: string;
  sort_order?: number;
  slug?: string;
  content?: string;
  image_ids?: unknown;
  original_price?: number;
  specs?: unknown;
  unit?: string;
  min_purchase?: number;
  max_purchase?: number;
  virtual_sales?: number;
  meta_title?: string;
  meta_description?: string;
  version: number;
}

export class AdminProducts {
  private readonly http: HttpClient;

  constructor(http: HttpClient) {
    this.http = http;
  }

  async list(
    page = 1,
    pageSize = 25,
    options?: RequestOptions,
  ): Promise<PaginatedData<ProductResponse>> {
    return this.http.get<PaginatedData<ProductResponse>>("/admin/products", {
      ...options,
      query: { page: String(page), page_size: String(pageSize) },
    });
  }

  async get(id: string, options?: RequestOptions): Promise<ProductResponse> {
    return this.http.get<ProductResponse>(`/admin/products/${id}`, options);
  }

  async create(
    body: CreateProductBody,
    options?: RequestOptions,
  ): Promise<ProductResponse> {
    return this.http.request<ProductResponse>(this.http.pathForCreate("/admin/products"), {
      ...options,
      method: this.http.methodForCreate(),
      body,
    });
  }

  async update(
    id: string,
    body: UpdateProductBody,
    options?: RequestOptions,
  ): Promise<ProductResponse> {
    return this.http.request<ProductResponse>(this.http.pathForUpdate("/admin/products", id), {
      ...options,
      method: this.http.methodForUpdate(),
      body,
    });
  }

  async delete(id: string, options?: RequestOptions): Promise<void> {
    await this.http.request<void>(this.http.pathForDelete("/admin/products", id), {
      ...options,
      method: this.http.methodForDelete(),
    });
  }
}
