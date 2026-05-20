import { HttpClient } from "../client";
import type { RequestOptions } from "../types";

export interface CartItem {
  id: string;
  quantity: number;
  attributes: string | null;
  title: string;
  price: number;
  cover_url: string | null;
  created_at: string;
  updated_at: string;
}

export interface CartResponse {
  items: CartItem[];
  total: number;
}

export interface AddToCartBody {
  product_id: string;
  quantity: number;
  attributes?: string;
}

export interface UpdateCartItemBody {
  quantity: number;
}

export class Cart {
  private readonly http: HttpClient;

  constructor(http: HttpClient) {
    this.http = http;
  }

  async list(options?: RequestOptions): Promise<CartResponse> {
    return this.http.get<CartResponse>("/cart", options);
  }

  async add(
    body: AddToCartBody,
    options?: RequestOptions,
  ): Promise<void> {
    await this.http.request<void>(this.http.pathForCreate("/cart"), {
      ...options,
      method: this.http.methodForCreate(),
      body,
    });
  }

  async updateItem(
    id: string,
    body: UpdateCartItemBody,
    options?: RequestOptions,
  ): Promise<void> {
    await this.http.request<void>(this.http.pathForUpdate("/cart", id), {
      ...options,
      method: this.http.methodForUpdate(),
      body,
    });
  }

  async removeItem(
    id: string,
    options?: RequestOptions,
  ): Promise<void> {
    await this.http.request<void>(this.http.pathForUpdate("/cart", id), {
      ...options,
      method: this.http.methodForUpdate(),
    });
  }

  async clear(options?: RequestOptions): Promise<void> {
    await this.http.request<void>("/cart", {
      ...options,
      method: "DELETE",
    });
  }

  async checkout(options?: RequestOptions): Promise<Record<string, unknown>> {
    return this.http.post<Record<string, unknown>>(
      "/cart/checkout",
      undefined,
      options,
    );
  }
}
