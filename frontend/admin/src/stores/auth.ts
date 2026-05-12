import { create } from "zustand";
import { persist } from "zustand/middleware";
import { UserRole } from "@raisfast/sdk";

interface User {
  id: string;
  email: string | null;
  username: string;
  role: UserRole;
  avatar: string | null;
  bio: string | null;
}

interface AuthState {
  user: User | null;
  accessToken: string | null;
  refreshToken: string | null;
  setTokens: (access: string, refresh: string) => void;
  setUser: (user: User) => void;
  login: (user: User, accessToken: string, refreshToken: string) => void;
  logout: () => void;
  isLoggedIn: () => boolean;
  isAdmin: () => boolean;
  isAuthor: () => boolean;
}

const SESSION_COOKIE = "session";

function setSessionCookie(token: string) {
  const secure = location.protocol === "https:" ? "; Secure" : "";
  document.cookie = `${SESSION_COOKIE}=${token}; path=/; max-age=${60 * 60 * 24 * 7}; SameSite=Lax${secure}`;
}

function clearSessionCookie() {
  document.cookie = `${SESSION_COOKIE}=; path=/; max-age=0`;
}

export const useAuthStore = create<AuthState>()(
  persist(
    (set, get) => ({
      user: null,
      accessToken: null,
      refreshToken: null,

      setTokens: (accessToken, refreshToken) =>
        set({ accessToken, refreshToken }),

      setUser: (user) => set({ user }),

      login: (user, accessToken, refreshToken) => {
        setSessionCookie(accessToken);
        set({ user, accessToken, refreshToken });
      },

      logout: () => {
        clearSessionCookie();
        set({ user: null, accessToken: null, refreshToken: null });
      },

      isLoggedIn: () => get().accessToken !== null,

      isAdmin: () => get().user?.role === UserRole.admin,

      isAuthor: () => {
        const role = get().user?.role;
        return role === UserRole.admin || role === UserRole.author;
      },
    }),
    { name: "auth-storage" },
  ),
);
