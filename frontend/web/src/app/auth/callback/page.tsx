"use client";

import { useEffect, useRef } from "react";
import { useRouter } from "next/navigation";
import { useAuthStore } from "@/stores/auth";
import { client } from "@/lib/raisfast";

export default function OAuthCallbackPage() {
  const router = useRouter();
  const login = useAuthStore((s) => s.login);
  const ran = useRef(false);

  useEffect(() => {
    if (ran.current) return;
    ran.current = true;

    const params = new URLSearchParams(window.location.search);
    const accessToken = params.get("access_token");
    const refreshToken = params.get("refresh_token");
    const error = params.get("error");

    if (error) {
      router.replace(`/auth/login?error=${encodeURIComponent(error)}`);
      return;
    }

    if (!accessToken) {
      router.replace("/auth/login?error=no_token");
      return;
    }

    (async () => {
      try {
        const user = await client.users.getMe();
        client.authStore.save({
          access_token: accessToken,
          refresh_token: refreshToken || "",
          user: {
            id: user.id,
            email: user.email,
            nickname: user.nickname,
            role: user.role,
            avatar: user.avatar,
            tenant_id: user.tenant_id,
          },
        });
        login(
          { id: user.id, email: user.email, username: user.nickname, role: user.role, avatar: user.avatar, bio: null },
          accessToken,
          refreshToken || "",
        );
        router.replace("/");
      } catch {
        router.replace("/auth/login?error=fetch_user_failed");
      }
    })();
  }, [router, login]);

  return (
    <div className="flex flex-col items-center justify-center gap-2">
      <div className="h-8 w-8 animate-spin rounded-full border-4 border-primary border-t-transparent" />
      <p className="text-sm text-muted-foreground">正在登录...</p>
    </div>
  );
}
