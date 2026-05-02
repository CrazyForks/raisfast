import { BaseAuthStore } from "@raisfast/sdk";
import { RaisFast } from "@raisfast/sdk";
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
        this._user = auth.user
          ? {
              id: auth.user.id,
              email: auth.user.email,
              nickname: auth.user.username,
              role: auth.user.role,
              avatar: auth.user.avatar,
              tenant_id: "default",
            }
          : null;
      }
    }
  }

  save(auth: {
    access_token: string;
    refresh_token: string;
    user: { id: string; email: string; nickname: string; role: string; avatar: string | null; tenant_id: string };
  }): void {
    super.save(auth);
    const store = useAuthStore.getState();
    store.setTokens(auth.access_token, auth.refresh_token);
    if (auth.user) {
      store.setUser({
        id: auth.user.id,
        email: auth.user.email,
        username: auth.user.nickname,
        role: auth.user.role,
        avatar: auth.user.avatar,
        bio: null,
      });
    }
  }

  clear(): void {
    super.clear();
    useAuthStore.getState().logout();
  }
}

const API_BASE = process.env.NEXT_PUBLIC_API_URL || "http://localhost:9000/api/v1";

export const client = new RaisFast(API_BASE, {
  authStore: new ZustandAuthStore(),
});

if (typeof window !== "undefined") {
  client.setTenantId(useTenantStore.getState().currentTenantId);

  useTenantStore.subscribe((state) => {
    client.setTenantId(state.currentTenantId);
  });
}
