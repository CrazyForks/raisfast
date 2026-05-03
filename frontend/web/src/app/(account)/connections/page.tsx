"use client";

import { useState, useEffect, useRef } from "react";
import { toast } from "sonner";

import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle, CardDescription } from "@/components/ui/card";
import { client } from "@/lib/raisfast";
import { SDKError } from "@raisfast/sdk";

interface OAuthBinding {
  id: string;
  provider: string;
  email: string | null;
  display_name: string | null;
  avatar_url: string | null;
  created_at: string;
}

export default function ConnectionsPage() {
  const [bindings, setBindings] = useState<OAuthBinding[]>([]);
  const [loading, setLoading] = useState(true);
  const [unlinking, setUnlinking] = useState<string | null>(null);
  const initialized = useRef(false);

  useEffect(() => {
    if (initialized.current) return;
    initialized.current = true;
    loadBindings();
  }, []);

  async function loadBindings() {
    try {
      const data = await client.send<OAuthBinding[]>("/auth/oauth/bindings");
      setBindings(data);
    } catch {
      // 可能没有绑定
    } finally {
      setLoading(false);
    }
  }

  async function handleUnbind(id: string, provider: string) {
    setUnlinking(id);
    try {
      await client.auth.unbindOAuth(provider);
      setBindings((prev) => prev.filter((b) => b.id !== id));
      toast.success(`${provider} account unlinked`);
    } catch (err) {
      if (err instanceof SDKError) {
        toast.error(err.message);
      } else {
        toast.error("Failed to unlink");
      }
    } finally {
      setUnlinking(null);
    }
  }

  const providerNames: Record<string, { name: string; color: string }> = {
    github: { name: "GitHub", color: "bg-gray-800 text-white" },
    google: { name: "Google", color: "bg-blue-600 text-white" },
    wechat: { name: "WeChat", color: "bg-green-500 text-white" },
  };

  return (
    <div className="space-y-6">
      <div>
        <h1 className="text-2xl font-bold">Connections</h1>
        <p className="text-muted-foreground">Manage linked social accounts.</p>
      </div>

      {loading ? (
        <div className="flex justify-center py-12">
          <div className="h-6 w-6 animate-spin rounded-full border-4 border-primary border-t-transparent" />
        </div>
      ) : bindings.length === 0 ? (
        <Card>
          <CardContent className="py-12 text-center text-muted-foreground">
            No social accounts linked.
          </CardContent>
        </Card>
      ) : (
        <div className="space-y-4">
          {bindings.map((binding, index) => {
            const info = providerNames[binding.provider] || {
              name: binding.provider,
              color: "bg-muted text-muted-foreground",
            };
            return (
              <Card key={binding.id ?? index}>
                <CardContent className="flex items-center justify-between py-4">
                  <div className="flex items-center gap-4">
                    <div className={`flex h-10 w-10 items-center justify-center rounded-lg text-sm font-medium ${info.color}`}>
                      {info.name.charAt(0)}
                    </div>
                    <div>
                      <p className="text-sm font-medium">{info.name}</p>
                      <p className="text-xs text-muted-foreground">
                        {binding.display_name || binding.email || "Connected"}
                      </p>
                    </div>
                  </div>
                  <Button
                    variant="outline"
                    size="sm"
                    disabled={unlinking === binding.id}
                    onClick={() => handleUnbind(binding.id, binding.provider)}
                  >
                    {unlinking === binding.id ? "Unlinking..." : "Unlink"}
                  </Button>
                </CardContent>
              </Card>
            );
          })}
        </div>
      )}
    </div>
  );
}
