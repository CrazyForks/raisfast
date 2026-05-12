import { HttpClient, toQueryString } from "../client";
import type {
  AdminWalletOperationRequest,
  BatchResponse,
  PaginatedData,
  RequestOptions,
  ReversalRequest,
  WalletResponse,
  WalletTransactionResponse,
} from "../types";

export class AdminWallets {
  private readonly http: HttpClient;

  constructor(http: HttpClient) {
    this.http = http;
  }

  async listWallets(
    query?: { page?: number; page_size?: number },
    options?: RequestOptions,
  ): Promise<PaginatedData<WalletResponse>> {
    return this.http.get<PaginatedData<WalletResponse>>("/admin/wallets", {
      ...options,
      query: toQueryString(query),
    });
  }

  async listTransactions(
    query?: { page?: number; page_size?: number },
    options?: RequestOptions,
  ): Promise<PaginatedData<WalletTransactionResponse>> {
    return this.http.get<PaginatedData<WalletTransactionResponse>>(
      "/admin/wallets/transactions",
      { ...options, query: query as unknown as Record<string, string> },
    );
  }

  async credit(
    body: AdminWalletOperationRequest,
    options?: RequestOptions,
  ): Promise<WalletTransactionResponse> {
    return this.http.post<WalletTransactionResponse>(
      "/admin/wallets/credit",
      body,
      options,
    );
  }

  async debit(
    body: AdminWalletOperationRequest,
    options?: RequestOptions,
  ): Promise<WalletTransactionResponse> {
    return this.http.post<WalletTransactionResponse>(
      "/admin/wallets/debit",
      body,
      options,
    );
  }

  async userTransactions(
    userId: string,
    query?: { page?: number; page_size?: number },
    options?: RequestOptions,
  ): Promise<PaginatedData<WalletTransactionResponse>> {
    return this.http.get<PaginatedData<WalletTransactionResponse>>(
      `/admin/wallets/${userId}/transactions`,
      { ...options, query: query as unknown as Record<string, string> },
    );
  }

  async userCurrencyTransactions(
    userId: string,
    currency: string,
    query?: { page?: number; page_size?: number },
    options?: RequestOptions,
  ): Promise<PaginatedData<WalletTransactionResponse>> {
    return this.http.get<PaginatedData<WalletTransactionResponse>>(
      `/admin/wallets/${userId}/${currency}/transactions`,
      { ...options, query: query as unknown as Record<string, string> },
    );
  }

  async reversal(
    txDocId: string,
    body?: ReversalRequest,
    options?: RequestOptions,
  ): Promise<WalletTransactionResponse> {
    return this.http.post<WalletTransactionResponse>(
      `/admin/wallets/${txDocId}/reversal`,
      body,
      options,
    );
  }
}
