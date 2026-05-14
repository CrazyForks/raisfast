import { HttpClient, toQueryString } from "../client";
import type {
  PaginatedData,
  PaymentChannelResponse,
  PaymentOrderResponse,
  PaymentRefundResponse,
  PaymentTransactionResponse,
  RequestOptions,
} from "../types";

export interface CreatePaymentChannelBody {
  provider: string;
  name: string;
  is_live?: boolean;
  credentials: string;
  webhook_secret?: string;
  settings?: string;
  sort_order?: number;
}

export interface UpdatePaymentChannelBody {
  name?: string;
  is_live?: boolean;
  credentials?: string;
  webhook_secret?: string;
  settings?: string;
  is_active?: boolean;
  sort_order?: number;
  version: number;
}

export interface CreateRefundBody {
  amount: number;
  reason?: string;
}

export class AdminPayment {
  private readonly http: HttpClient;

  constructor(http: HttpClient) {
    this.http = http;
  }

  async listChannels(
    page = 1,
    pageSize = 25,
    options?: RequestOptions,
  ): Promise<PaginatedData<PaymentChannelResponse>> {
    return this.http.get<PaginatedData<PaymentChannelResponse>>("/admin/payment/channels", {
      ...options,
      query: { page: String(page), page_size: String(pageSize) },
    });
  }

  async getChannel(
    id: string,
    options?: RequestOptions,
  ): Promise<PaymentChannelResponse> {
    return this.http.get<PaymentChannelResponse>(`/admin/payment/channels/${id}`, options);
  }

  async createChannel(
    body: CreatePaymentChannelBody,
    options?: RequestOptions,
  ): Promise<PaymentChannelResponse> {
    return this.http.request<PaymentChannelResponse>(
      this.http.pathForCreate("/admin/payment/channels"),
      { ...options, method: this.http.methodForCreate(), body },
    );
  }

  async updateChannel(
    id: string,
    body: UpdatePaymentChannelBody,
    options?: RequestOptions,
  ): Promise<PaymentChannelResponse> {
    return this.http.request<PaymentChannelResponse>(
      this.http.pathForUpdate("/admin/payment/channels", id),
      { ...options, method: this.http.methodForUpdate(), body },
    );
  }

  async deleteChannel(id: string, options?: RequestOptions): Promise<void> {
    await this.http.request<void>(this.http.pathForDelete("/admin/payment/channels", id), {
      ...options,
      method: this.http.methodForDelete(),
    });
  }

  async listOrders(
    query?: { page?: number; page_size?: number },
    options?: RequestOptions,
  ): Promise<PaginatedData<PaymentOrderResponse>> {
    return this.http.get<PaginatedData<PaymentOrderResponse>>("/admin/payment/orders", {
      ...options,
      query: toQueryString(query),
    });
  }

  async getOrder(
    id: string,
    options?: RequestOptions,
  ): Promise<PaymentOrderResponse> {
    return this.http.get<PaymentOrderResponse>(`/admin/payment/orders/${id}`, options);
  }

  async refundOrder(
    id: string,
    body: CreateRefundBody,
    options?: RequestOptions,
  ): Promise<PaymentRefundResponse> {
    return this.http.post<PaymentRefundResponse>(
      `/admin/payment/orders/${id}/refund`,
      body,
      options,
    );
  }

  async listTransactions(
    query?: { page?: number; page_size?: number },
    options?: RequestOptions,
  ): Promise<PaginatedData<PaymentTransactionResponse>> {
    return this.http.get<PaginatedData<PaymentTransactionResponse>>(
      "/admin/payment/transactions",
      { ...options, query: toQueryString(query) },
    );
  }

  async listRefunds(
    query?: { page?: number; page_size?: number },
    options?: RequestOptions,
  ): Promise<PaginatedData<PaymentRefundResponse>> {
    return this.http.get<PaginatedData<PaymentRefundResponse>>(
      "/admin/payment/refunds",
      { ...options, query: toQueryString(query) },
    );
  }
}
