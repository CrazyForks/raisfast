import { BaseAuthStore, RaisFast, UserRole } from "@raisfast/sdk";
import { useAuthStore } from "@/stores/auth";
import { useTenantStore } from "@/stores/tenant";

class ZustandAuthStore extends BaseAuthStore {
  private unsubAuth: (() => void) | null = null;
  private unsubTenant: (() => void) | null = null;

  constructor() {
    super();

    if (typeof window !== "undefined") {
      const auth = useAuthStore.getState();
      if (auth.accessToken) {
        this._token = auth.accessToken;
        this._refreshToken = auth.refreshToken;
      }
    }
  }

  save(auth: {
    access_token: string;
    refresh_token: string;
    expires_in?: number | bigint | null;
    user: Record<string, unknown>;
  }): void {
    this._token = auth.access_token;
    this._refreshToken = auth.refresh_token;
    this._user = auth.user as never;
    this._notify();
    const store = useAuthStore.getState();
    store.setTokens(auth.access_token, auth.refresh_token);
    if (auth.user) {
      const u = auth.user as Record<string, unknown>;
      store.setUser({
        id: String(u.id),
        email: null,
        username: String(u.username ?? ""),
        role: String(u.role ?? "") as UserRole,
        avatar: typeof u.avatar === "string" ? u.avatar : null,
        bio: typeof u.bio === "string" ? u.bio : null,
      });
    }
  }

  clear(): void {
    super.clear();
    useAuthStore.getState().logout();
  }
}

const API_BASE = import.meta.env.VITE_API_URL || "http://localhost:9898/api/v1";

export const client = new RaisFast(API_BASE, {
  authStore: new ZustandAuthStore(),
});

if (typeof window !== "undefined") {
  client.setTenantId(useTenantStore.getState().currentTenantId);

  useTenantStore.subscribe((state) => {
    client.setTenantId(state.currentTenantId);
  });
}
