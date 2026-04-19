"use client";

import { useState } from "react";
import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import {
  Puzzle,
  Power,
  PowerOff,
  RefreshCw,
  Trash2,
  AlertTriangle,
  Activity,
  ExternalLink,
} from "lucide-react";
import { toast } from "sonner";
import Link from "next/link";

import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { Card, CardContent } from "@/components/ui/card";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
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

interface PluginItem {
  id: string;
  name: string;
  version: string;
  description: string;
  runtime: string;
  enabled: boolean;
  health: PluginHealth;
  hooks: string[];
  metrics: Record<string, PluginMetrics>;
}

interface PaginatedData<T> {
  items: T[];
  total: number;
  page: number;
  page_size: number;
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

export default function PluginsPage() {
  const { t } = useT();
  const queryClient = useQueryClient();
  const [page, setPage] = useState(1);
  const pageSize = 20;

  const pluginsQuery = useQuery({
    queryKey: ["plugins", page],
    queryFn: () =>
      api.get<PaginatedData<PluginItem>>(`/admin/plugins?page=${page}&page_size=${pageSize}`),
  });

  const enableMutation = useMutation({
    mutationFn: (id: string) => api.post(`/admin/plugins/${id}/enable`, {}),
    onSuccess: () => {
      toast.success(t("plugins.pluginEnabled"));
      queryClient.invalidateQueries({ queryKey: ["plugins"] });
    },
    onError: (err) => {
      toast.error(err instanceof ApiError ? err.message : t("plugins.failedToEnable"));
    },
  });

  const disableMutation = useMutation({
    mutationFn: (id: string) => api.post(`/admin/plugins/${id}/disable`, {}),
    onSuccess: () => {
      toast.success(t("plugins.pluginDisabled"));
      queryClient.invalidateQueries({ queryKey: ["plugins"] });
    },
    onError: (err) => {
      toast.error(err instanceof ApiError ? err.message : t("plugins.failedToDisable"));
    },
  });

  const reloadMutation = useMutation({
    mutationFn: (id: string) => api.post(`/admin/plugins/${id}/reload`, {}),
    onSuccess: () => {
      toast.success(t("plugins.pluginReloaded"));
      queryClient.invalidateQueries({ queryKey: ["plugins"] });
    },
    onError: (err) => {
      toast.error(err instanceof ApiError ? err.message : t("plugins.failedToReload"));
    },
  });

  const removeMutation = useMutation({
    mutationFn: (id: string) => api.delete(`/admin/plugins/${id}`),
    onSuccess: () => {
      toast.success(t("plugins.pluginRemoved"));
      queryClient.invalidateQueries({ queryKey: ["plugins"] });
    },
    onError: (err) => {
      toast.error(err instanceof ApiError ? err.message : t("plugins.failedToRemove"));
    },
  });

  const plugins = pluginsQuery.data?.items ?? [];
  const totalPages = Math.ceil((pluginsQuery.data?.total ?? 0) / pageSize);

  return (
    <div className="space-y-6">
      <div className="flex items-center justify-between">
        <h1 className="text-2xl font-bold">{t("plugins.title")}</h1>
        <Badge variant="outline">{t("plugins.loaded", { count: plugins.length })}</Badge>
      </div>

      <Card>
        <CardContent className="p-0">
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead>{t("plugins.plugin")}</TableHead>
                <TableHead>{t("plugins.runtime")}</TableHead>
                <TableHead>{t("common.status")}</TableHead>
                <TableHead>{t("plugins.hooks")}</TableHead>
                <TableHead>{t("plugins.health")}</TableHead>
                <TableHead className="text-right">{t("common.actions")}</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {pluginsQuery.isLoading ? (
                <TableRow>
                  <TableCell colSpan={6} className="text-center py-8">
                    {t("common.loading")}
                  </TableCell>
                </TableRow>
              ) : plugins.length === 0 ? (
                <TableRow>
                  <TableCell colSpan={6} className="text-center py-8">
                    <div className="flex flex-col items-center gap-2 text-muted-foreground">
                      <Puzzle className="size-8" />
                      <p>{t("plugins.noPlugins")}</p>
                      <p className="text-xs">
                        {t("plugins.placePlugins")}
                      </p>
                    </div>
                  </TableCell>
                </TableRow>
              ) : (
                plugins.map((p) => {
                  const rb = runtimeBadge(p.runtime);
                  const totalCalls = Object.values(p.metrics).reduce(
                    (s, m) => s + m.total_calls,
                    0,
                  );
                  const totalErrors = Object.values(p.metrics).reduce(
                    (s, m) => s + m.total_errors,
                    0,
                  );
                  const totalDuration = Object.values(p.metrics).reduce(
                    (s, m) => s + m.total_duration_us,
                    0,
                  );

                  return (
                    <TableRow key={p.id}>
                      <TableCell>
                        <Link
                          href={`/admin/plugins/${encodeURIComponent(p.id)}`}
                          className="space-y-0.5 block group"
                        >
                          <div className="font-medium group-hover:underline">
                            {p.name}
                          </div>
                          <div className="text-xs text-muted-foreground">
                            {p.id} &middot; v{p.version}
                          </div>
                          {p.description && (
                            <div className="text-xs text-muted-foreground max-w-xs truncate">
                              {p.description}
                            </div>
                          )}
                        </Link>
                      </TableCell>
                      <TableCell>
                        <Badge variant={rb.variant}>{rb.label}</Badge>
                      </TableCell>
                      <TableCell>
                        {p.enabled ? (
                          <Badge variant="default">{t("common.enabled")}</Badge>
                        ) : (
                          <Badge variant="outline">{t("common.disabled")}</Badge>
                        )}
                      </TableCell>
                      <TableCell>
                        <div className="flex flex-wrap gap-1">
                          {p.hooks.length === 0 ? (
                            <span className="text-xs text-muted-foreground">
                              {t("common.none")}
                            </span>
                          ) : (
                            p.hooks.slice(0, 3).map((h) => (
                              <Badge key={h} variant="ghost" className="text-xs">
                                {h}
                              </Badge>
                            ))
                          )}
                          {p.hooks.length > 3 && (
                            <Badge variant="ghost" className="text-xs">
                              +{p.hooks.length - 3}
                            </Badge>
                          )}
                        </div>
                      </TableCell>
                      <TableCell>
                        {p.health.auto_disabled ? (
                          <Tooltip>
                            <TooltipTrigger>
                              <div className="flex items-center gap-1 text-destructive">
                                <AlertTriangle className="size-4" />
                                <span className="text-xs font-medium">
                                  {t("plugins.autoDisabled")}
                                </span>
                              </div>
                            </TooltipTrigger>
                            <TooltipContent>
                              {p.health.last_error || t("plugins.tooManyErrors")}
                            </TooltipContent>
                          </Tooltip>
                        ) : p.health.error_count > 0 ? (
                          <Tooltip>
                            <TooltipTrigger>
                              <div className="flex items-center gap-1 text-yellow-600">
                                <AlertTriangle className="size-4" />
                                <span className="text-xs">
                                  {t("plugins.errorCount", { count: p.health.error_count })}
                                </span>
                              </div>
                            </TooltipTrigger>
                            <TooltipContent>
                              {p.health.last_error || t("plugins.errorsOccurred")}
                            </TooltipContent>
                          </Tooltip>
                        ) : totalCalls > 0 ? (
                          <div className="flex items-center gap-1 text-muted-foreground">
                            <Activity className="size-3.5" />
                            <span className="text-xs">
                              {t("plugins.calls", { count: totalCalls })} &middot;{" "}
                              {formatDuration(totalDuration)}
                            </span>
                          </div>
                        ) : (
                          <span className="text-xs text-muted-foreground">
                            {t("plugins.idle")}
                          </span>
                        )}
                      </TableCell>
                      <TableCell className="text-right">
                        <div className="flex items-center justify-end gap-1">
                          <Link href={`/admin/plugins/${encodeURIComponent(p.id)}`}>
                            <Button
                              variant="ghost"
                              size="icon-sm"
                              title="View details"
                            >
                              <ExternalLink className="size-4" />
                            </Button>
                          </Link>
                          {p.enabled ? (
                            <Button
                              variant="ghost"
                              size="icon-sm"
                              title="Disable"
                              disabled={disableMutation.isPending}
                              onClick={() => disableMutation.mutate(p.id)}
                            >
                              <PowerOff className="size-4" />
                            </Button>
                          ) : (
                            <Button
                              variant="ghost"
                              size="icon-sm"
                              title="Enable"
                              disabled={enableMutation.isPending}
                              onClick={() => enableMutation.mutate(p.id)}
                            >
                              <Power className="size-4" />
                            </Button>
                          )}
                          <Button
                            variant="ghost"
                            size="icon-sm"
                            title="Reload"
                            disabled={reloadMutation.isPending}
                            onClick={() => reloadMutation.mutate(p.id)}
                          >
                            <RefreshCw className="size-4" />
                          </Button>
                          <Button
                            variant="ghost"
                            size="icon-sm"
                            title="Remove"
                            disabled={removeMutation.isPending}
                            onClick={() => {
                              if (
                                confirm(
                                  t("plugins.confirmRemove", { name: p.name })
                                )
                              ) {
                                removeMutation.mutate(p.id);
                              }
                            }}
                          >
                            <Trash2 className="size-4 text-destructive" />
                          </Button>
                        </div>
                      </TableCell>
                    </TableRow>
                  );
                })
              )}
            </TableBody>
          </Table>
        </CardContent>
      </Card>

      {totalPages > 1 && (
        <div className="flex items-center justify-center gap-2">
          <Button
            variant="outline"
            size="sm"
            disabled={page <= 1}
            onClick={() => setPage((p) => p - 1)}
          >
            {t("common.previous")}
          </Button>
          <span className="text-sm text-muted-foreground">
            {t("common.pageOf", { page, total: totalPages })}
          </span>
          <Button
            variant="outline"
            size="sm"
            disabled={page >= totalPages}
            onClick={() => setPage((p) => p + 1)}
          >
            {t("common.next")}
          </Button>
        </div>
      )}
    </div>
  );
}
