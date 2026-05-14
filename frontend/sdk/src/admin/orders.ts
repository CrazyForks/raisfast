import { HttpClient, toQueryString } from "../client";
import type {
  OrderResponse,
  OrderStatsResponse,
  PaginatedData,
  RequestOptions,
} from "../types";

export interface ShipOrderBody {
  tracking_no?: string;
  carrier?: string;
}

export interface UpdateAdminRemarkBody {
  admin_remark: string;
}

export class AdminOrders {
  private readonly http: HttpClient;

  constructor(http: HttpClient) {
    this.http = http;
  }

  async list(
    query?: { page?: number; page_size?: number; status?: string },
    options?: RequestOptions,
  ): Promise<PaginatedData<OrderResponse>> {
    return this.http.get<PaginatedData<OrderResponse>>("/admin/orders", {
      ...options,
      query: toQueryString(query as Record<string, string | number | undefined>),
    });
  }

  async get(id: string, options?: RequestOptions): Promise<OrderResponse> {
    return this.http.get<OrderResponse>(`/admin/orders/${id}`, options);
  }

  async pay(id: string, options?: RequestOptions): Promise<void> {
    await this.http.post(`/admin/orders/${id}/pay`, undefined, options);
  }

  async ship(id: string, body: ShipOrderBody, options?: RequestOptions): Promise<void> {
    await this.http.post(`/admin/orders/${id}/ship`, body, options);
  }

  async cancel(id: string, options?: RequestOptions): Promise<void> {
    await this.http.post(`/admin/orders/${id}/cancel`, undefined, options);
  }

  async refund(id: string, options?: RequestOptions): Promise<void> {
    await this.http.post(`/admin/orders/${id}/refund`, undefined, options);
  }

  async updateRemark(
    id: string,
    body: UpdateAdminRemarkBody,
    options?: RequestOptions,
  ): Promise<void> {
    await this.http.request<void>(this.http.pathForUpdate("/admin/orders", `${id}/remark`), {
      ...options,
      method: this.http.methodForUpdate(),
      body,
    });
  }

  async stats(options?: RequestOptions): Promise<OrderStatsResponse> {
    return this.http.get<OrderStatsResponse>("/admin/orders/stats", options);
  }
}
