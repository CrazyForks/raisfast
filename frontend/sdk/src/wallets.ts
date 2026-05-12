import { HttpClient } from "./client";
import type {
  AdminWalletOperationRequest,
  PaginatedData,
  ReversalRequest,
  RequestOptions,
  WalletResponse,
  WalletTransactionResponse,
} from "./types";

export class Wallets {
  private readonly http: HttpClient;

  constructor(http: HttpClient) {
    this.http = http;
  }

  async list(options?: RequestOptions): Promise<WalletResponse[]> {
    return this.http.get<WalletResponse[]>("/wallets", options);
  }

  async get(currency: string, options?: RequestOptions): Promise<WalletResponse> {
    return this.http.get<WalletResponse>(`/wallets/${currency}`, options);
  }

  async listTransactions(
    currency: string,
    query?: { page?: number; page_size?: number },
    options?: RequestOptions,
  ): Promise<PaginatedData<WalletTransactionResponse>> {
    return this.http.get<PaginatedData<WalletTransactionResponse>>(
      `/wallets/${currency}/transactions`,
      { ...options, query: query as unknown as Record<string, string> },
    );
  }

  async listAllTransactions(
    query?: { page?: number; page_size?: number },
    options?: RequestOptions,
  ): Promise<PaginatedData<WalletTransactionResponse>> {
    return this.http.get<PaginatedData<WalletTransactionResponse>>(
      "/wallets/transactions",
      { ...options, query: query as unknown as Record<string, string> },
    );
  }

  async adminListWallets(
    query?: { page?: number; page_size?: number },
    options?: RequestOptions,
  ): Promise<PaginatedData<WalletResponse>> {
    return this.http.get<PaginatedData<WalletResponse>>(
      "/admin/wallets",
      { ...options, query: query as unknown as Record<string, string> },
    );
  }

  async adminListTransactions(
    query?: { page?: number; page_size?: number },
    options?: RequestOptions,
  ): Promise<PaginatedData<WalletTransactionResponse>> {
    return this.http.get<PaginatedData<WalletTransactionResponse>>(
      "/admin/wallets/transactions",
      { ...options, query: query as unknown as Record<string, string> },
    );
  }

  async adminCredit(
    body: AdminWalletOperationRequest,
    options?: RequestOptions,
  ): Promise<WalletTransactionResponse> {
    return this.http.post<WalletTransactionResponse>(
      "/admin/wallets/credit",
      body,
      options,
    );
  }

  async adminDebit(
    body: AdminWalletOperationRequest,
    options?: RequestOptions,
  ): Promise<WalletTransactionResponse> {
    return this.http.post<WalletTransactionResponse>(
      "/admin/wallets/debit",
      body,
      options,
    );
  }

  async adminUserTransactions(
    userId: string,
    query?: { page?: number; page_size?: number },
    options?: RequestOptions,
  ): Promise<PaginatedData<WalletTransactionResponse>> {
    return this.http.get<PaginatedData<WalletTransactionResponse>>(
      `/admin/wallets/${userId}/transactions`,
      { ...options, query: query as unknown as Record<string, string> },
    );
  }

  async adminUserCurrencyTransactions(
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

  async adminReversal(
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
