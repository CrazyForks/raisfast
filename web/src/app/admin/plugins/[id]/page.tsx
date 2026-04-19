"use client";

import { useRouter, useParams } from "next/navigation";
import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import Link from "next/link";
import {
  ArrowLeft,
  Power,
  PowerOff,
  RefreshCw,
  Trash2,
  AlertTriangle,
  Activity,
  Puzzle,
  Clock,
  Zap,
  Shield,
  FileText,
  Lock,
} from "lucide-react";
import { toast } from "sonner";

import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import {
  Card,
  CardContent,
  CardHeader,
  CardTitle,
  CardDescription,
} from "@/components/ui/card";
import { Separator } from "@/components/ui/separator";
import { Skeleton } from "@/components/ui/skeleton";
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@/components/ui/tooltip";
import { api, ApiError } from "@/lib/api";
import { useT } from "@/lib/i18n";

interface PluginHealth {
  error_count: number;
  last_error: string | null;
  last_error_at: string | null;
  auto_disabled: boolean;
}

interface PluginMetrics {
  total_calls: number;
  total_errors: number;
  total_duration_us: number;
}

interface PluginPermissions {
  http: string[];
  config: string[];
  database: string[];
  filesystem: string[];
  max_memory_mb: number | null;
  timeout_ms: number | null;
}

interface PluginDetail {
  id: string;
  name: string;
  version: string;
  description: string;
  runtime: string;
  enabled: boolean;
  health: PluginHealth;
  hooks: string[];
  metrics: Record<string, PluginMetrics>;
  permissions: PluginPermissions;
}

function runtimeBadge(runtime: string) {
  switch (runtime) {
    case "wasm":
      return { variant: "default" as const, label: "WASM" };
    case "js":
      return { variant: "secondary" as const, label: "JS" };
    case "lua":
      return { variant: "outline" as const, label: "Lua" };
    default:
      return { variant: "outline" as const, label: runtime };
  }
}

function formatDuration(us: number): string {
  if (us < 1000) return `${us}µs`;
  if (us < 1_000_000) return `${(us / 1000).toFixed(1)}ms`;
  return `${(us / 1_000_000).toFixed(2)}s`;
}

function InfoRow({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <div className="flex items-start justify-between py-2">
      <span className="text-sm text-muted-foreground">{label}</span>
      <div className="text-sm text-right">{children}</div>
    </div>
  );
}

export default function PluginDetailPage() {
  const { t } = useT();
  const router = useRouter();
  const params = useParams();
  const queryClient = useQueryClient();
  const id = decodeURIComponent(params.id as string);

  const pluginQuery = useQuery({
    queryKey: ["plugin", id],
    queryFn: () => api.get<PluginDetail>(`/admin/plugins/${encodeURIComponent(id)}`),
    retry: false,
  });

  const enableMutation = useMutation({
    mutationFn: () => api.post(`/admin/plugins/${encodeURIComponent(id)}/enable`, {}),
    onSuccess: () => {
      toast.success(t("plugins.pluginEnabled"));
      queryClient.invalidateQueries({ queryKey: ["plugin", id] });
    },
    onError: (err) => {
      toast.error(err instanceof ApiError ? err.message : t("plugins.failedToEnable"));
    },
  });

  const disableMutation = useMutation({
    mutationFn: () => api.post(`/admin/plugins/${encodeURIComponent(id)}/disable`, {}),
    onSuccess: () => {
      toast.success(t("plugins.pluginDisabled"));
      queryClient.invalidateQueries({ queryKey: ["plugin", id] });
    },
    onError: (err) => {
      toast.error(err instanceof ApiError ? err.message : t("plugins.failedToDisable"));
    },
  });

  const reloadMutation = useMutation({
    mutationFn: () => api.post(`/admin/plugins/${encodeURIComponent(id)}/reload`, {}),
    onSuccess: () => {
      toast.success(t("plugins.pluginReloaded"));
      queryClient.invalidateQueries({ queryKey: ["plugin", id] });
    },
    onError: (err) => {
      toast.error(err instanceof ApiError ? err.message : t("plugins.failedToReload"));
    },
  });

  const removeMutation = useMutation({
    mutationFn: () => api.delete(`/admin/plugins/${encodeURIComponent(id)}`),
    onSuccess: () => {
      toast.success(t("plugins.pluginRemoved"));
      queryClient.invalidateQueries({ queryKey: ["plugins"] });
      router.push("/admin/plugins");
    },
    onError: (err) => {
      toast.error(err instanceof ApiError ? err.message : t("plugins.failedToRemove"));
    },
  });

  if (pluginQuery.isLoading) {
    return (
      <div className="space-y-6">
        <div className="flex items-center gap-4">
          <Link href="/admin/plugins">
            <Button variant="outline" size="sm">
              &larr; {t("common.back")}
            </Button>
          </Link>
          <Skeleton className="h-8 w-48" />
        </div>
        <div className="grid gap-6 md:grid-cols-2">
          <Card>
            <CardContent className="pt-6 space-y-4">
              <Skeleton className="h-5 w-32" />
              <Skeleton className="h-5 w-full" />
              <Skeleton className="h-5 w-full" />
              <Skeleton className="h-5 w-24" />
            </CardContent>
          </Card>
          <Card>
            <CardContent className="pt-6 space-y-4">
              <Skeleton className="h-5 w-32" />
              <Skeleton className="h-5 w-full" />
              <Skeleton className="h-5 w-full" />
            </CardContent>
          </Card>
        </div>
      </div>
    );
  }

  if (pluginQuery.error || !pluginQuery.data) {
    return (
      <div className="space-y-6">
        <div className="flex items-center gap-4">
          <Link href="/admin/plugins">
            <Button variant="outline" size="sm">
              &larr; {t("common.back")}
            </Button>
          </Link>
          <h1 className="text-2xl font-bold">{t("plugins.notFound")}</h1>
        </div>
        <Card>
          <CardContent className="pt-6">
            <div className="flex flex-col items-center gap-2 py-8 text-muted-foreground">
              <Puzzle className="size-8" />
              <p>
                {pluginQuery.error instanceof ApiError
                  ? pluginQuery.error.message
                  : t("plugins.notFoundMsg")}
              </p>
              <Link href="/admin/plugins">
                <Button variant="outline" size="sm">
                  {t("plugins.backToPlugins")}
                </Button>
              </Link>
            </div>
          </CardContent>
        </Card>
      </div>
    );
  }

  const plugin = pluginQuery.data;
  const perms = plugin?.permissions ?? {
    http: [],
    config: [],
    database: [],
    filesystem: [],
    max_memory_mb: null,
    timeout_ms: null,
  };
  const rb = runtimeBadge(plugin.runtime);
  const totalCalls = Object.values(plugin.metrics).reduce(
    (s, m) => s + m.total_calls,
    0,
  );
  const totalErrors = Object.values(plugin.metrics).reduce(
    (s, m) => s + m.total_errors,
    0,
  );
  const totalDuration = Object.values(plugin.metrics).reduce(
    (s, m) => s + m.total_duration_us,
    0,
  );

  return (
    <div className="space-y-6">
      <div className="flex items-center justify-between">
        <div className="flex items-center gap-4">
          <Link href="/admin/plugins">
            <Button variant="outline" size="sm">
              &larr; {t("common.back")}
            </Button>
          </Link>
          <div>
            <h1 className="text-2xl font-bold">{plugin.name}</h1>
            <p className="text-sm text-muted-foreground">{plugin.id}</p>
          </div>
        </div>
        <div className="flex items-center gap-2">
          {plugin.enabled ? (
            <Button
              variant="outline"
              size="sm"
              disabled={disableMutation.isPending}
              onClick={() => disableMutation.mutate()}
            >
              <PowerOff className="size-4" />
              {t("common.disable")}
            </Button>
          ) : (
            <Button
              variant="outline"
              size="sm"
              disabled={enableMutation.isPending}
              onClick={() => enableMutation.mutate()}
            >
              <Power className="size-4" />
              {t("common.enable")}
            </Button>
          )}
          <Button
            variant="outline"
            size="sm"
            disabled={reloadMutation.isPending}
            onClick={() => reloadMutation.mutate()}
          >
            <RefreshCw className="size-4" />
            {t("plugins.reload")}
          </Button>
          <Button
            variant="outline"
            size="sm"
            className="text-destructive hover:text-destructive"
            disabled={removeMutation.isPending}
            onClick={() => {
              if (
                confirm(
                  t("plugins.confirmRemove", { name: plugin.name })
                )
              ) {
                removeMutation.mutate();
              }
            }}
          >
            <Trash2 className="size-4" />
            {t("plugins.remove")}
          </Button>
        </div>
      </div>

      <div className="grid gap-6 md:grid-cols-2">
        <Card>
          <CardHeader>
            <CardTitle className="flex items-center gap-2">
              <FileText className="size-4" />
              {t("plugins.general")}
            </CardTitle>
          </CardHeader>
          <CardContent className="space-y-0">
            <InfoRow label="ID">{plugin.id}</InfoRow>
            <Separator />
            <InfoRow label="Name">{plugin.name}</InfoRow>
            <Separator />
            <InfoRow label={t("plugins.version")}>{plugin.version}</InfoRow>
            <Separator />
            <InfoRow label={t("plugins.runtime")}>
              <Badge variant={rb.variant}>{rb.label}</Badge>
            </InfoRow>
            <Separator />
            <InfoRow label={t("common.status")}>
              {plugin.enabled ? (
                <Badge variant="default">{t("common.enabled")}</Badge>
              ) : (
                <Badge variant="outline">{t("common.disabled")}</Badge>
              )}
            </InfoRow>
            {plugin.description && (
              <>
                <Separator />
                <InfoRow label={t("common.description")}>
                  <span className="max-w-xs">{plugin.description}</span>
                </InfoRow>
              </>
            )}
          </CardContent>
        </Card>

        <Card>
          <CardHeader>
            <CardTitle className="flex items-center gap-2">
              <Shield className="size-4" />
              {t("plugins.health")}
            </CardTitle>
            <CardDescription>
              {t("plugins.errorTracking")}
            </CardDescription>
          </CardHeader>
          <CardContent className="space-y-0">
            <InfoRow label={t("common.status")}>
              {plugin.health.auto_disabled ? (
                <Badge variant="destructive">{t("plugins.autoDisabled")}</Badge>
              ) : plugin.health.error_count > 0 ? (
                <Badge variant="secondary">
                  {t("plugins.errorCount", { count: plugin.health.error_count })}
                </Badge>
              ) : (
                <Badge variant="default">{t("plugins.healthy")}</Badge>
              )}
            </InfoRow>
            <Separator />
            <InfoRow label={t("plugins.errorCountLabel")}>
              {plugin.health.error_count}
            </InfoRow>
            {plugin.health.last_error && (
              <>
                <Separator />
                <InfoRow label={t("plugins.lastError")}>
                  <Tooltip>
                    <TooltipTrigger>
                      <span className="max-w-xs truncate text-destructive block">
                        {plugin.health.last_error}
                      </span>
                    </TooltipTrigger>
                    <TooltipContent className="max-w-sm">
                      {plugin.health.last_error}
                    </TooltipContent>
                  </Tooltip>
                </InfoRow>
              </>
            )}
            {plugin.health.last_error_at && (
              <>
                <Separator />
                <InfoRow label={t("plugins.lastErrorAt")}>
                  {new Date(plugin.health.last_error_at).toLocaleString()}
                </InfoRow>
              </>
            )}
            <Separator />
            <InfoRow label={t("plugins.autoDisabledLabel")}>
              {plugin.health.auto_disabled ? t("field.yes") : t("field.no")}
            </InfoRow>
          </CardContent>
        </Card>
      </div>

      <Card>
        <CardHeader>
          <CardTitle className="flex items-center gap-2">
            <Lock className="size-4" />
            {t("plugins.permissions")}
          </CardTitle>
          <CardDescription>
            {t("permissions.declaredFromManifest")}
          </CardDescription>
        </CardHeader>
        <CardContent className="space-y-0">
          <InfoRow label="HTTP">
            {perms.http.length > 0 ? (
              <div className="flex flex-wrap gap-1 justify-end">
                {perms.http.map((p) => (
                  <Badge key={p} variant="ghost" className="text-xs">
                    {p}
                  </Badge>
                ))}
              </div>
            ) : (
              <span className="text-muted-foreground">{t("common.none")}</span>
            )}
          </InfoRow>
          <Separator />
          <InfoRow label="Config">
            {perms.config.length > 0 ? (
              <div className="flex flex-wrap gap-1 justify-end">
                {perms.config.map((p) => (
                  <Badge key={p} variant="ghost" className="text-xs">
                    {p}
                  </Badge>
                ))}
              </div>
            ) : (
              <span className="text-muted-foreground">{t("common.none")}</span>
            )}
          </InfoRow>
          <Separator />
          <InfoRow label="Database">
            {perms.database.length > 0 ? (
              <div className="flex flex-wrap gap-1 justify-end">
                {perms.database.map((p) => (
                  <Badge key={p} variant="ghost" className="text-xs">
                    {p}
                  </Badge>
                ))}
              </div>
            ) : (
              <span className="text-muted-foreground">{t("common.none")}</span>
            )}
          </InfoRow>
          <Separator />
          <InfoRow label="Filesystem">
            {perms.filesystem.length > 0 ? (
              <div className="flex flex-wrap gap-1 justify-end">
                {perms.filesystem.map((p) => (
                  <Badge key={p} variant="ghost" className="text-xs">
                    {p}
                  </Badge>
                ))}
              </div>
            ) : (
              <span className="text-muted-foreground">{t("common.none")}</span>
            )}
          </InfoRow>
          <Separator />
          <InfoRow label={t("plugins.maxMemory")}>
            {perms.max_memory_mb != null
              ? `${perms.max_memory_mb} MB`
              : t("plugins.default")}
          </InfoRow>
          <Separator />
          <InfoRow label={t("plugins.timeout")}>
            {perms.timeout_ms != null
              ? `${perms.timeout_ms} ms`
              : t("plugins.default")}
          </InfoRow>
        </CardContent>
      </Card>

      <Card>
        <CardHeader>
          <CardTitle className="flex items-center gap-2">
            <Zap className="size-4" />
            {t("plugins.hooksTitle")}
          </CardTitle>
          <CardDescription>
            {t("plugins.hooksDesc")}
          </CardDescription>
        </CardHeader>
        <CardContent>
          {plugin.hooks.length === 0 ? (
            <p className="text-sm text-muted-foreground py-4">
              {t("plugins.noHooks")}
            </p>
          ) : (
            <div className="space-y-3">
              {plugin.hooks.map((hook) => {
                const m = plugin.metrics[hook];
                const avgUs =
                  m && m.total_calls > 0
                    ? Math.round(m.total_duration_us / m.total_calls)
                    : null;
                return (
                  <div key={hook}>
                    <div className="flex items-center justify-between py-2">
                      <div className="flex items-center gap-2">
                        <Badge variant="ghost">{hook}</Badge>
                        {m && m.total_errors > 0 && (
                          <span className="text-xs text-destructive">
                            {t("plugins.errorCount", { count: m.total_errors })}
                          </span>
                        )}
                      </div>
                      {m ? (
                        <div className="flex items-center gap-4 text-xs text-muted-foreground">
                          <span className="flex items-center gap-1">
                            <Activity className="size-3" />
                            {m.total_calls} calls
                          </span>
                          <span className="flex items-center gap-1">
                            <Clock className="size-3" />
                            total {formatDuration(m.total_duration_us)}
                          </span>
                          {avgUs !== null && (
                            <span>avg {formatDuration(avgUs)}</span>
                          )}
                        </div>
                      ) : (
                        <span className="text-xs text-muted-foreground">
                          {t("plugins.noMetrics")}
                        </span>
                      )}
                    </div>
                    <Separator />
                  </div>
                );
              })}
            </div>
          )}
        </CardContent>
      </Card>

      <Card>
        <CardHeader>
          <CardTitle className="flex items-center gap-2">
            <Activity className="size-4" />
            {t("plugins.performance")}
          </CardTitle>
        </CardHeader>
        <CardContent>
          <div className="grid gap-4 sm:grid-cols-3">
            <div className="rounded-lg border p-4 text-center">
              <div className="text-2xl font-bold">{totalCalls}</div>
              <div className="text-xs text-muted-foreground">{t("plugins.totalCalls")}</div>
            </div>
            <div className="rounded-lg border p-4 text-center">
              <div className="text-2xl font-bold">
                {totalErrors > 0 ? (
                  <span className="text-destructive">{totalErrors}</span>
                ) : (
                  <span>{totalErrors}</span>
                )}
              </div>
              <div className="text-xs text-muted-foreground">{t("plugins.totalErrors")}</div>
            </div>
            <div className="rounded-lg border p-4 text-center">
              <div className="text-2xl font-bold">
                {formatDuration(totalDuration)}
              </div>
              <div className="text-xs text-muted-foreground">{t("plugins.totalDuration")}</div>
            </div>
          </div>
        </CardContent>
      </Card>
    </div>
  );
}
