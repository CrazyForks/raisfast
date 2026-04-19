"use client";

import { useState } from "react";
import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import Link from "next/link";
import { useRouter } from "next/navigation";
import {
  Package,
  Power,
  PowerOff,
  Trash2,
  Layers,
  Puzzle,
  AlertTriangle,
  RefreshCw,
  ExternalLink,
  MoreVertical,
} from "lucide-react";
import { toast } from "sonner";

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
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Checkbox } from "@/components/ui/checkbox";
import { DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuTrigger } from "@/components/ui/dropdown-menu";
import { api, apiRequest, ApiError } from "@/lib/api";
import { useT } from "@/lib/i18n";

interface ExtensionItem {
  id: string;
  name: string;
  version: string;
  description: string;
  enabled: boolean;
  installed: boolean;
  has_content_types: boolean;
  has_plugin: boolean;
  content_types: string[];
  dependencies: Record<string, string>;
  installed_at: string | null;
}

export default function ExtensionsPage() {
  const queryClient = useQueryClient();
  const router = useRouter();
  const { t } = useT();
  const [uninstallTarget, setUninstallTarget] = useState<ExtensionItem | null>(null);
  const [dropTables, setDropTables] = useState(false);

  const listQuery = useQuery({
    queryKey: ["extensions"],
    queryFn: () => api.get<ExtensionItem[]>("/admin/extensions"),
  });

  const enableMutation = useMutation({
    mutationFn: (id: string) => api.post(`/admin/extensions/${id}/enable`, {}),
    onSuccess: () => {
      toast.success(t("extensions.extensionEnabled"));
      queryClient.invalidateQueries({ queryKey: ["extensions"] });
    },
    onError: (err) => {
      toast.error(err instanceof ApiError ? err.message : t("extensions.failedToEnable"));
    },
  });

  const disableMutation = useMutation({
    mutationFn: (id: string) => api.post(`/admin/extensions/${id}/disable`, {}),
    onSuccess: () => {
      toast.success(t("extensions.extensionDisabled"));
      queryClient.invalidateQueries({ queryKey: ["extensions"] });
    },
    onError: (err) => {
      toast.error(err instanceof ApiError ? err.message : t("extensions.failedToDisable"));
    },
  });

  const uninstallMutation = useMutation({
    mutationFn: ({ id, drop }: { id: string; drop: boolean }) =>
      apiRequest<void>(`/admin/extensions/${id}`, {
        method: "DELETE",
        body: JSON.stringify({ drop_tables: drop }),
      }),
    onSuccess: () => {
      toast.success(t("extensions.extensionUninstalled"));
      setUninstallTarget(null);
      setDropTables(false);
      queryClient.invalidateQueries({ queryKey: ["extensions"] });
    },
    onError: (err) => {
      toast.error(err instanceof ApiError ? err.message : t("extensions.failedToUninstall"));
    },
  });

  const extensions = listQuery.data ?? [];

  const enabledCount = extensions.filter((e) => e.enabled).length;
  const disabledCount = extensions.length - enabledCount;

  return (
    <div className="space-y-6">
      <div className="flex items-center justify-between">
        <h1 className="text-2xl font-bold">{t("extensions.title")}</h1>
        <div className="flex items-center gap-2">
          <Badge variant="default">{t("common.enabledCount", { count: enabledCount })}</Badge>
          {disabledCount > 0 && (
            <Badge variant="outline">{t("common.disabledCount", { count: disabledCount })}</Badge>
          )}
        </div>
      </div>

      <Card>
        <CardContent className="p-0">
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead>{t("extensions.extension")}</TableHead>
                <TableHead>{t("extensions.components")}</TableHead>
                <TableHead>{t("extensions.contentTypesCol")}</TableHead>
                <TableHead>{t("extensions.dependencies")}</TableHead>
                <TableHead>{t("common.status")}</TableHead>
                <TableHead className="text-right">{t("common.actions")}</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {listQuery.isLoading ? (
                <TableRow>
                  <TableCell colSpan={6} className="text-center py-8">
                    <div className="flex items-center justify-center gap-2">
                      <RefreshCw className="size-4 animate-spin" />
                      {t("common.loading")}
                    </div>
                  </TableCell>
                </TableRow>
              ) : listQuery.isError ? (
                <TableRow>
                  <TableCell colSpan={6} className="text-center py-8">
                    <div className="flex flex-col items-center gap-2 text-destructive">
                      <AlertTriangle className="size-8" />
                      <p>{t("extensions.failedToLoad")}</p>
                      <Button
                        variant="outline"
                        size="sm"
                        onClick={() => queryClient.invalidateQueries({ queryKey: ["extensions"] })}
                      >
                        {t("common.retry")}
                      </Button>
                    </div>
                  </TableCell>
                </TableRow>
              ) : extensions.length === 0 ? (
                <TableRow>
                  <TableCell colSpan={6} className="text-center py-8">
                    <div className="flex flex-col items-center gap-2 text-muted-foreground">
                      <Package className="size-8" />
                      <p>{t("extensions.noExtensions")}</p>
                      <p className="text-xs">
                        {t("extensions.placeExtensions")}
                      </p>
                    </div>
                  </TableCell>
                </TableRow>
              ) : (
                extensions.map((ext) => {
                  const depKeys = Object.keys(ext.dependencies);

                  return (
                    <TableRow key={ext.id}>
                      <TableCell>
                        <Link
                          href={`/admin/extensions/${encodeURIComponent(ext.id)}`}
                          className="space-y-0.5 block group"
                        >
                          <div className="font-medium group-hover:underline">
                            {ext.name}
                          </div>
                          <div className="text-xs text-muted-foreground">
                            {ext.id} &middot; v{ext.version}
                          </div>
                          {ext.description && (
                            <div className="text-xs text-muted-foreground max-w-xs truncate">
                              {ext.description}
                            </div>
                          )}
                        </Link>
                      </TableCell>
                      <TableCell>
                        <div className="flex items-center gap-1.5">
                          {ext.has_content_types && (
                            <Tooltip>
                              <TooltipTrigger>
                                <Badge variant="secondary" className="gap-1">
                                  <Layers className="size-3" />
                                  CT
                                </Badge>
                              </TooltipTrigger>
                              <TooltipContent>
                                {t("extensions.containsContentTypesCount", { count: ext.content_types.length })}
                              </TooltipContent>
                            </Tooltip>
                          )}
                          {ext.has_plugin && (
                            <Tooltip>
                              <TooltipTrigger>
                                <Badge variant="outline" className="gap-1">
                                  <Puzzle className="size-3" />
                                  {t("layout.plugins")}
                                </Badge>
                              </TooltipTrigger>
                              <TooltipContent>
                                {t("extensions.containsPlugin")}
                              </TooltipContent>
                            </Tooltip>
                          )}
                          {!ext.has_content_types && !ext.has_plugin && (
                            <span className="text-xs text-muted-foreground">—</span>
                          )}
                        </div>
                      </TableCell>
                      <TableCell>
                        {ext.content_types.length > 0 ? (
                          <div className="flex flex-wrap gap-1">
                            {ext.content_types.map((ct) => (
                              <Badge key={ct} variant="ghost" className="text-xs">
                                {ct}
                              </Badge>
                            ))}
                          </div>
                        ) : (
                          <span className="text-xs text-muted-foreground">—</span>
                        )}
                      </TableCell>
                      <TableCell>
                        {depKeys.length > 0 ? (
                          <div className="flex flex-wrap gap-1">
                            {depKeys.map((dep) => (
                              <Tooltip key={dep}>
                                <TooltipTrigger>
                                  <Badge variant="ghost" className="text-xs">
                                    {dep}
                                  </Badge>
                                </TooltipTrigger>
                                <TooltipContent>
                                  {dep} {ext.dependencies[dep]}
                                </TooltipContent>
                              </Tooltip>
                            ))}
                          </div>
                        ) : (
                          <span className="text-xs text-muted-foreground">{t("common.none")}</span>
                        )}
                      </TableCell>
                      <TableCell>
                        {ext.enabled ? (
                          <Badge variant="default">{t("common.enabled")}</Badge>
                        ) : (
                          <Badge variant="outline">{t("common.disabled")}</Badge>
                        )}
                      </TableCell>
                      <TableCell className="text-right">
                        <DropdownMenu>
                          <DropdownMenuTrigger className="inline-flex items-center justify-center rounded-md p-1 hover:bg-muted transition-colors">
                            <MoreVertical className="size-4" />
                          </DropdownMenuTrigger>
                          <DropdownMenuContent align="end">
                            <DropdownMenuItem onClick={() => router.push(`/admin/extensions/${encodeURIComponent(ext.id)}`)}>
                              <ExternalLink className="size-4" />
                              View details
                            </DropdownMenuItem>
                            {ext.enabled ? (
                              <DropdownMenuItem
                                disabled={disableMutation.isPending}
                                onClick={() => disableMutation.mutate(ext.id)}
                              >
                                <PowerOff className="size-4" />
                                Disable
                              </DropdownMenuItem>
                            ) : (
                              <DropdownMenuItem
                                disabled={enableMutation.isPending}
                                onClick={() => enableMutation.mutate(ext.id)}
                              >
                                <Power className="size-4" />
                                Enable
                              </DropdownMenuItem>
                            )}
                            <DropdownMenuItem
                              onClick={() => {
                                setUninstallTarget(ext);
                                setDropTables(false);
                              }}
                            >
                              <Trash2 className="size-4 text-destructive" />
                              Uninstall
                            </DropdownMenuItem>
                          </DropdownMenuContent>
                        </DropdownMenu>
                      </TableCell>
                    </TableRow>
                  );
                })
              )}
            </TableBody>
          </Table>
        </CardContent>
      </Card>

      <Dialog
        open={!!uninstallTarget}
        onOpenChange={(open) => {
          if (!open) setUninstallTarget(null);
        }}
      >
        <DialogContent>
          <DialogHeader>
            <DialogTitle>{t("extensions.uninstallExtension")}</DialogTitle>
            <DialogDescription>
              {t("extensions.uninstallConfirm", { name: uninstallTarget?.name ?? "", version: uninstallTarget?.version ?? "" })}
            </DialogDescription>
          </DialogHeader>
          {uninstallTarget && uninstallTarget.content_types.length > 0 && (
            <div className="flex items-start gap-2 rounded-md border border-destructive/50 bg-destructive/5 p-3">
              <AlertTriangle className="size-4 mt-0.5 text-destructive shrink-0" />
              <div className="space-y-2">
                <p className="text-sm">
                  {t("extensions.containsContentTypes", { types: uninstallTarget.content_types.join(", ") })}
                </p>
                <div className="flex items-center gap-2">
                  <Checkbox
                    id="drop-tables"
                    checked={dropTables}
                    onCheckedChange={(checked) => setDropTables(checked === true)}
                  />
                  <label htmlFor="drop-tables" className="text-sm font-medium">
                    {t("extensions.dropTables")}
                  </label>
                </div>
              </div>
            </div>
          )}
          <DialogFooter>
            <Button
              variant="outline"
              onClick={() => setUninstallTarget(null)}
              disabled={uninstallMutation.isPending}
            >
              {t("common.cancel")}
            </Button>
            <Button
              variant="destructive"
              disabled={uninstallMutation.isPending}
              onClick={() => {
                if (uninstallTarget) {
                  uninstallMutation.mutate({
                    id: uninstallTarget.id,
                    drop: dropTables,
                  });
                }
              }}
            >
              {uninstallMutation.isPending ? t("extensions.uninstalling") : t("extensions.uninstall")}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </div>
  );
}
