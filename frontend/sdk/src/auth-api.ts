import { HttpClient } from "./client";
import type {
  AuthConfig,
  AuthResult,
  CredentialResponse,
  IAuthStore,
  OAuthBinding,
  OAuthProvider,
  RequestOptions,
  User,
} from "./types";

export class Auth {
  private readonly http: HttpClient;
  private readonly store: IAuthStore;

  constructor(http: HttpClient, store: IAuthStore) {
    this.http = http;
    this.store = store;
  }

  get token(): string | null {
    return this.store.token;
  }

  get user(): User | null {
    return this.store.user;
  }

  get isAuthenticated(): boolean {
    return this.store.isAuthenticated;
  }

  async login(
    email: string,
    password: string,
    options?: RequestOptions,
  ): Promise<AuthResult> {
    const result = await this.http.post<AuthResult>(
      "/auth/login",
      { email, password },
      options,
    );
    this.store.save(result);
    return result;
  }

  async register(
    data: { email: string; password: string; username: string },
    options?: RequestOptions,
  ): Promise<AuthResult> {
    const result = await this.http.post<AuthResult>(
      "/auth/register",
      data,
      options,
    );
    this.store.save(result);
    return result;
  }

  async refresh(options?: RequestOptions): Promise<AuthResult | null> {
    const rt = this.store.refreshToken;
    if (!rt) return null;

    const result = await this.http.post<AuthResult>(
      "/auth/refresh",
      { refresh_token: rt },
      options,
    );
    this.store.save(result);
    return result;
  }

  async logout(options?: RequestOptions): Promise<void> {
    try {
      await this.http.post("/auth/logout", undefined, options);
    } finally {
      this.store.clear();
    }
  }

  async getConfig(options?: RequestOptions): Promise<AuthConfig> {
    return this.http.get<AuthConfig>("/auth/config", options);
  }

  async getMe(options?: RequestOptions): Promise<User> {
    return this.http.get<User>("/users/me", options);
  }

  async updateMe(
    data: { username?: string; bio?: string; website?: string; avatar?: string; social_links?: Record<string, string>; metadata?: unknown },
    options?: RequestOptions,
  ): Promise<User> {
    return this.http.put<User>("/users/me", data, options);
  }

  async changePassword(
    data: { old_password: string; new_password: string },
    options?: RequestOptions,
  ): Promise<void> {
    await this.http.put("/users/me/password", data, options);
  }

  async requestPasswordReset(
    email: string,
    options?: RequestOptions,
  ): Promise<void> {
    await this.http.post("/auth/forgot-password", { email }, options);
  }

  async confirmPasswordReset(
    data: { token: string; new_password: string },
    options?: RequestOptions,
  ): Promise<void> {
    await this.http.post("/auth/reset-password", data, options);
  }

  async setPassword(
    data: { email: string; new_password: string },
    options?: RequestOptions,
  ): Promise<void> {
    await this.http.post("/auth/set-password", data, options);
  }

  async verifyEmail(
    data: { token: string },
    options?: RequestOptions,
  ): Promise<void> {
    await this.http.post("/auth/verify-email", data, options);
  }

  async resendVerification(
    email: string,
    options?: RequestOptions,
  ): Promise<void> {
    await this.http.post("/auth/resend-verification", { email }, options);
  }

  async sendSmsCode(
    phone: string,
    purpose: string,
    options?: RequestOptions,
  ): Promise<void> {
    await this.http.post("/auth/sms/send", { phone, purpose }, options);
  }

  async verifySms(
    data: { phone: string; code: string; purpose: string },
    options?: RequestOptions,
  ): Promise<AuthResult> {
    const result = await this.http.post<AuthResult>(
      "/auth/sms/verify",
      data,
      options,
    );
    this.store.save(result);
    return result;
  }

  async bindPhone(
    data: { phone: string; code: string },
    options?: RequestOptions,
  ): Promise<void> {
    await this.http.post("/auth/phone/bind", data, options);
  }

  // ─── OAuth2 ───

  getOAuthRedirectURL(provider: string): string {
    return `${this.http.baseUrl}/auth/oauth/${provider}`;
  }

  async listOAuthProviders(options?: RequestOptions): Promise<OAuthProvider[]> {
    return this.http.get<OAuthProvider[]>(
      "/auth/oauth/providers",
      options,
    );
  }

  async authWithOAuth(
    data: { provider: string; code: string; redirect_url: string },
    options?: RequestOptions,
  ): Promise<AuthResult> {
    const result = await this.http.get<AuthResult>(
      `/auth/oauth/${data.provider}/callback`,
      {
        ...options,
        query: { code: data.code, redirect_url: data.redirect_url },
      },
    );
    this.store.save(result);
    return result;
  }

  async listOAuthBindings(options?: RequestOptions): Promise<OAuthBinding[]> {
    return this.http.get<OAuthBinding[]>(
      "/auth/oauth/bindings",
      options,
    );
  }

  async unbindOAuth(
    provider: string,
    options?: RequestOptions,
  ): Promise<void> {
    await this.http.del(`/auth/oauth/${provider}/unbind`, options);
  }

  async listCredentials(options?: RequestOptions): Promise<CredentialResponse[]> {
    return this.http.get<CredentialResponse[]>("/auth/credentials", options);
  }

  async bindEmail(
    data: { email: string; password: string },
    options?: RequestOptions,
  ): Promise<void> {
    await this.http.post("/auth/credentials/bind-email", data, options);
  }

  async deleteCredential(id: number, options?: RequestOptions): Promise<void> {
    await this.http.del(`/auth/credentials/${id}`, options);
  }
}
