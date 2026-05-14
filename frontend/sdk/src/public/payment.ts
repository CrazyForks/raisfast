import { HttpClient, toQueryString } from "../client";
import type {
  AvailableChannelsResponse,
  PaymentOrderResponse,
  PaymentRefundResponse,
  PaymentTransactionResponse,
  PaginatedData,
  RequestOptions,
} from "../types";

export interface CreatePaymentOrderBody {
  order_id: string;
  channel_id?: string;
  method?: string;
  country?: string;
  language?: string;
  return_url?: string;
  metadata?: string;
}

export class Payment {
  private readonly http: HttpClient;

  constructor(http: HttpClient) {
    this.http = http;
  }

  async listAvailableChannels(
    query: { order_id: string; country?: string; language?: string },
    options?: RequestOptions,
  ): Promise<AvailableChannelsResponse> {
    return this.http.get<AvailableChannelsResponse>("/payment/channels/available", {
      ...options,
      query: toQueryString(query),
    });
  }

  async listOrders(
    page = 1,
    pageSize = 25,
    options?: RequestOptions,
  ): Promise<PaginatedData<PaymentOrderResponse>> {
    return this.http.get<PaginatedData<PaymentOrderResponse>>("/payment/orders", {
      ...options,
      query: { page: String(page), page_size: String(pageSize) },
    });
  }

  async createOrder(
    body: CreatePaymentOrderBody,
    options?: RequestOptions,
  ): Promise<PaymentOrderResponse> {
    return this.http.request<PaymentOrderResponse>(this.http.pathForCreate("/payment/orders"), {
      ...options,
      method: this.http.methodForCreate(),
      body,
    });
  }

  async getOrder(
    id: string,
    options?: RequestOptions,
  ): Promise<PaymentOrderResponse> {
    return this.http.get<PaymentOrderResponse>(`/payment/orders/${id}`, options);
  }

  async cancelOrder(id: string, options?: RequestOptions): Promise<void> {
    await this.http.post(`/payment/orders/${id}/cancel`, undefined, options);
  }

  async listTransactions(
    id: string,
    options?: RequestOptions,
  ): Promise<PaymentTransactionResponse[]> {
    return this.http.get<PaymentTransactionResponse[]>(
      `/payment/orders/${id}/transactions`,
      options,
    );
  }

  async listRefunds(
    id: string,
    options?: RequestOptions,
  ): Promise<PaymentRefundResponse[]> {
    return this.http.get<PaymentRefundResponse[]>(
      `/payment/orders/${id}/refunds`,
      options,
    );
  }
}
