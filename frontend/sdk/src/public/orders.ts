import { HttpClient } from "../client";
import type { OrderResponse, PaginatedData, RequestOptions } from "../types";

export interface CreateOrderItemBody {
  product_id: string;
  quantity: number;
}

export interface CreateOrderBody {
  items: CreateOrderItemBody[];
  currency?: string;
  buyer_name?: string;
  buyer_phone?: string;
  buyer_email?: string;
  shipping_address?: string;
  remark?: string;
}

export class Orders {
  private readonly http: HttpClient;

  constructor(http: HttpClient) {
    this.http = http;
  }

  async list(
    page = 1,
    pageSize = 25,
    options?: RequestOptions,
  ): Promise<PaginatedData<OrderResponse>> {
    return this.http.get<PaginatedData<OrderResponse>>("/orders", {
      ...options,
      query: { page: String(page), page_size: String(pageSize) },
    });
  }

  async get(id: string, options?: RequestOptions): Promise<OrderResponse> {
    return this.http.get<OrderResponse>(`/orders/${id}`, options);
  }

  async create(
    body: CreateOrderBody,
    options?: RequestOptions,
  ): Promise<OrderResponse> {
    return this.http.request<OrderResponse>(this.http.pathForCreate("/orders"), {
      ...options,
      method: this.http.methodForCreate(),
      body,
    });
  }

  async cancel(id: string, options?: RequestOptions): Promise<void> {
    await this.http.request<void>(this.http.pathForUpdate("/orders", id), {
      ...options,
      method: this.http.methodForUpdate(),
    });
  }

  async confirmReceipt(id: string, options?: RequestOptions): Promise<void> {
    await this.http.post(`/orders/${id}/confirm`, undefined, options);
  }
}
