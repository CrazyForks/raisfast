import { HttpClient, toQueryString } from "../client";
import type {
  PaginatedData,
  RequestOptions,
  WalletResponse,
  WalletTransactionResponse,
} from "../types";

export class Wallets {
  private readonly http: HttpClient;

  constructor(http: HttpClient) {
    this.http = http;
  }

  async list(options?: RequestOptions): Promise<WalletResponse[]> {
    return this.http.get<WalletResponse[]>("/wallets", options);
  }

  async get(
    currency: string,
    options?: RequestOptions,
  ): Promise<WalletResponse> {
    return this.http.get<WalletResponse>(`/wallets/${currency}`, options);
  }

  async listTransactions(
    currency: string,
    query?: { page?: number; page_size?: number },
    options?: RequestOptions,
  ): Promise<PaginatedData<WalletTransactionResponse>> {
    return this.http.get<PaginatedData<WalletTransactionResponse>>(
      `/wallets/${currency}/transactions`,
      { ...options, query: toQueryString(query) },
    );
  }

  async listAllTransactions(
    query?: { page?: number; page_size?: number },
    options?: RequestOptions,
  ): Promise<PaginatedData<WalletTransactionResponse>> {
    return this.http.get<PaginatedData<WalletTransactionResponse>>(
      "/wallets/transactions",
      { ...options, query: toQueryString(query) },
    );
  }
}
