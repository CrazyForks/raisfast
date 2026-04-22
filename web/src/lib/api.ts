import { useAuthStore } from "@/stores/auth";
import { useTenantStore } from "@/stores/tenant";

const API_BASE = process.env.NEXT_PUBLIC_API_URL || "http://localhost:9000/api/v1";

export interface Post {
  id: string;
  title: string;
  slug: string;
  content: string;
  excerpt: string;
  cover_image: string;
  status: string;
  author_id: string;
  author_name: string;
  category_id: string;
  category_name: string;
  tags: { id: string; name: string; slug: string }[];
  view_count: number;
  is_pinned: boolean;
  created_at: string;
  updated_at: string;
  published_at: string;
  title_highlight: string | null;
  excerpt_highlight: string | null;
}

export interface PaginatedData<T> {
  items: T[];
  total: number;
  page: number;
  page_size: number;
}

export interface MediaFile {
  id: string;
  user_id: string;
  filename: string;
  url: string;
  mimetype: string;
  size: number;
  width: number | null;
  height: number | null;
  created_at: string;
}

export interface Comment {
  id: string;
  content: string;
  author_name: string;
  nickname: string;
  created_at: string;
  replies?: Comment[];
}

export class ApiError extends Error {
  code: number;
  constructor(code: number, message: string) {
    super(message);
    this.code = code;
  }
}

async function refreshToken(): Promise<string | null> {
  const store = useAuthStore.getState();
  if (!store.refreshToken) return null;

  const res = await fetch(`${API_BASE}/auth/refresh`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ refresh_token: store.refreshToken }),
  });

  if (!res.ok) {
    store.logout();
    return null;
  }

  const json = await res.json();
  if (json.code !== 0) {
    store.logout();
    return null;
  }

  store.setTokens(json.data.access_token, json.data.refresh_token);
  return json.data.access_token;
}

export async function apiRequest<T>(
  path: string,
  options: RequestInit = {},
  overrideToken?: string,
): Promise<T> {
  const store = useAuthStore.getState();
  const locale =
    typeof navigator !== "undefined" ? navigator.language : "en";

  const headers = new Headers(options.headers);
  headers.set("Content-Type", "application/json");
  headers.set("Accept-Language", locale);

  const token = overrideToken || store.accessToken;
  if (token) {
    headers.set("Authorization", `Bearer ${token}`);
  }

  const tenantStore = useTenantStore.getState();
  if (tenantStore.currentTenantId) {
    headers.set("X-Tenant-ID", tenantStore.currentTenantId);
  }

  let res = await fetch(`${API_BASE}${path}`, { ...options, headers });

  if (res.status === 401 && store.refreshToken) {
    const newToken = await refreshToken();
    if (newToken) {
      headers.set("Authorization", `Bearer ${newToken}`);
      res = await fetch(`${API_BASE}${path}`, { ...options, headers });
    }
  }

  const json = await res.json();

  if (json.code !== 0) {
    throw new ApiError(json.code, json.message);
  }

  return json.data as T;
}

export const api = {
  get: <T>(path: string, token?: string) => apiRequest<T>(path, {}, token),

  post: <T>(path: string, body: unknown) =>
    apiRequest<T>(path, {
      method: "POST",
      body: JSON.stringify(body),
    }),

  put: <T>(path: string, body: unknown) =>
    apiRequest<T>(path, {
      method: "PUT",
      body: JSON.stringify(body),
    }),

  delete: <T>(path: string) => apiRequest<T>(path, { method: "DELETE" }),

  upload: async <T>(path: string, file: File): Promise<T> => {
    const store = useAuthStore.getState();
    const formData = new FormData();
    formData.append("file", file);

    const res = await fetch(`${API_BASE}${path}`, {
      method: "POST",
      headers: {
        Authorization: `Bearer ${store.accessToken || ""}`,
      },
      body: formData,
    });

    const json = await res.json();
    if (json.code !== 0) {
      throw new ApiError(json.code, json.message);
    }
    return json.data as T;
  },
};
