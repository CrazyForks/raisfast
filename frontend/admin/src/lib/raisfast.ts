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
    expires_in?: number;
    user: { id: string; username: string; role: string; avatar: string | null; bio: string | null; website: string | null; created_at: string; updated_at: string; status?: string; registered_via?: string };
  }): void {
    super.save({ ...auth, expires_in: auth.expires_in ?? 0 } as never);
    const store = useAuthStore.getState();
    store.setTokens(auth.access_token, auth.refresh_token);
    if (auth.user) {
        store.setUser({
        id: auth.user.id,
        email: null,
        username: auth.user.username,
        role: auth.user.role as UserRole,
        avatar: auth.user.avatar,
        bio: auth.user.bio,
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
