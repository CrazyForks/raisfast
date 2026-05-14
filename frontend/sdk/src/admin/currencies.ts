import { HttpClient } from "../client";
import type {
  CreateCurrencyRequest,
  CurrencyResponse,
  RequestOptions,
  UpdateCurrencyRequest,
} from "../types";

export class AdminCurrencies {
  private readonly http: HttpClient;

  constructor(http: HttpClient) {
    this.http = http;
  }

  async list(options?: RequestOptions): Promise<CurrencyResponse[]> {
    return this.http.get<CurrencyResponse[]>("/admin/currencies", options);
  }

  async get(
    code: string,
    options?: RequestOptions,
  ): Promise<CurrencyResponse> {
    return this.http.get<CurrencyResponse>(
      `/admin/currencies/${code}`,
      options,
    );
  }

  async create(
    body: CreateCurrencyRequest,
    options?: RequestOptions,
  ): Promise<CurrencyResponse> {
    return this.http.request<CurrencyResponse>(this.http.pathForCreate("/admin/currencies"), {
      ...options,
      method: this.http.methodForCreate(),
      body,
    });
  }

  async update(
    code: string,
    body: UpdateCurrencyRequest,
    options?: RequestOptions,
  ): Promise<CurrencyResponse> {
    return this.http.request<CurrencyResponse>(this.http.pathForUpdate("/admin/currencies", code), {
      ...options,
      method: this.http.methodForUpdate(),
      body,
    });
  }
}
