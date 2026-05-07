
import { useEffect, useState } from "react";
import { Button } from "@/components/ui/button";
import { Separator } from "@/components/ui/separator";
import { client } from "@/lib/raisfast";
import type { OAuthProvider } from "@raisfast/sdk";

const API_BASE =
  import.meta.env.VITE_API_URL || "http://localhost:9898/api/v1";

const providerLabels: Record<string, { label: string; icon: ReactNode }> = {
  github: {
    label: "GitHub",
    icon: (
      <svg className="h-4 w-4" viewBox="0 0 24 24" fill="currentColor">
        <path d="M12 0C5.37 0 0 5.37 0 12c0 5.31 3.435 9.795 8.205 11.385.6.105.825-.255.825-.57 0-.285-.015-1.23-.015-2.235-3.015.555-3.795-.735-4.035-1.41-.135-.345-.72-1.41-1.23-1.695-.42-.225-1.02-.78-.015-.795.945-.015 1.62.87 1.845 1.23 1.08 1.815 2.805 1.305 3.495.99.105-.78.42-1.305.765-1.605-2.67-.3-5.46-1.335-5.46-5.925 0-1.305.465-2.385 1.23-3.225-.12-.3-.54-1.53.12-3.18 0 0 1.005-.315 3.3 1.23.96-.27 1.98-.405 3-.405s2.04.135 3 .405c2.295-1.56 3.3-1.23 3.3-1.23.66 1.65.24 2.88.12 3.18.765.84 1.23 1.905 1.23 3.225 0 4.605-2.805 5.625-5.475 5.925.435.375.81 1.095.81 2.22 0 1.605-.015 2.895-.015 3.3 0 .315.225.69.825.57A12.02 12.02 0 0024 12c0-6.63-5.37-12-12-12z" />
      </svg>
    ),
  },
  google: {
    label: "Google",
    icon: (
      <svg className="h-4 w-4" viewBox="0 0 24 24">
        <path
          d="M22.56 12.25c0-.78-.07-1.53-.2-2.25H12v4.26h5.92a5.06 5.06 0 01-2.2 3.32v2.77h3.57c2.08-1.92 3.28-4.74 3.28-8.1z"
          fill="#4285F4"
        />
        <path
          d="M12 23c2.97 0 5.46-.98 7.28-2.66l-3.57-2.77c-.98.66-2.23 1.06-3.71 1.06-2.86 0-5.29-1.93-6.16-4.53H2.18v2.84C3.99 20.53 7.7 23 12 23z"
          fill="#34A853"
        />
        <path
          d="M5.84 14.09c-.22-.66-.35-1.36-.35-2.09s.13-1.43.35-2.09V7.07H2.18C1.43 8.55 1 10.22 1 12s.43 3.45 1.18 4.93l2.85-2.22.81-.62z"
          fill="#FBBC05"
        />
        <path
          d="M12 5.38c1.62 0 3.06.56 4.21 1.64l3.15-3.15C17.45 2.09 14.97 1 12 1 7.7 1 3.99 3.47 2.18 7.07l3.66 2.84c.87-2.6 3.3-4.53 6.16-4.53z"
          fill="#EA4335"
        />
      </svg>
    ),
  },
  wechat: {
    label: "WeChat",
    icon: (
      <svg className="h-4 w-4" viewBox="0 0 24 24" fill="#07C160">
        <path d="M8.691 2.188C3.891 2.188 0 5.476 0 9.53c0 2.212 1.17 4.203 3.002 5.55a.59.59 0 01.213.665l-.39 1.48c-.019.07-.048.141-.048.213 0 .163.13.295.29.295a.326.326 0 00.167-.054l1.903-1.114a.864.864 0 01.717-.098 10.16 10.16 0 002.837.403c.276 0 .543-.027.811-.05a6.13 6.13 0 01-.253-1.728c0-3.572 3.193-6.468 7.13-6.468.234 0 .463.022.693.04C16.418 4.695 12.891 2.188 8.691 2.188zm-2.37 4.053a1.1 1.1 0 110 2.2 1.1 1.1 0 010-2.2zm4.742 0a1.1 1.1 0 110 2.2 1.1 1.1 0 010-2.2zM23.907 14.63c0-3.349-3.276-6.065-7.316-6.065-4.038 0-7.312 2.716-7.312 6.065 0 3.35 3.274 6.065 7.312 6.065.832 0 1.633-.118 2.384-.334a.723.723 0 01.601.082l1.598.935a.272.272 0 00.14.046.247.247 0 00.243-.247c0-.06-.024-.12-.04-.179l-.327-1.233a.493.493 0 01.178-.558c1.536-1.124 2.539-2.814 2.539-4.577zm-9.785-1.278a.923.923 0 110 1.846.923.923 0 010-1.846zm4.938 0a.923.923 0 110 1.846.923.923 0 010-1.846z" />
      </svg>
    ),
  },
};

type ReactNode = React.ReactNode;

export function OAuthButtons() {
  const [providers, setProviders] = useState<OAuthProvider[]>([]);

  useEffect(() => {
    client.auth.listOAuthProviders()
      .then((data) => {
        const list = Array.isArray(data) ? data : [];
        setProviders(list);
      })
      .catch(() => {});
  }, []);

  if (providers.length === 0) return null;

  return (
    <>
      <div className="flex flex-col gap-2">
        {providers.map((p) => {
          const info = providerLabels[p.name];
          if (!info) return null;
          return (
            <a key={p.name} href={`${API_BASE}/auth/oauth/${p.name}`}>
              <Button
                type="button"
                variant="outline"
                className="w-full gap-2"
              >
                {info.icon}
                Continue with {info.label}
              </Button>
            </a>
          );
        })}
      </div>

      <div className="flex items-center gap-3 my-1">
        <Separator className="flex-1" />
        <span className="text-xs text-muted-foreground">or</span>
        <Separator className="flex-1" />
      </div>
    </>
  );
}
