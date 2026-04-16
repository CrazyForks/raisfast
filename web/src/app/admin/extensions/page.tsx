"use client";

import { useState } from "react";
import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import Link from "next/link";
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
import { api, apiRequest, ApiError } from "@/lib/api";

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
  const [uninstallTarget, setUninstallTarget] = useState<ExtensionItem | null>(null);
  const [dropTables, setDropTables] = useState(false);

  const listQuery = useQuery({
    queryKey: ["extensions"],
    queryFn: () => api.get<ExtensionItem[]>("/admin/extensions"),
  });

  const enableMutation = useMutation({
    mutationFn: (id: string) => api.post(`/admin/extensions/${id}/enable`, {}),
    onSuccess: () => {
      toast.success("Extension enabled");
      queryClient.invalidateQueries({ queryKey: ["extensions"] });
    },
    onError: (err) => {
      toast.error(err instanceof ApiError ? err.message : "Failed to enable extension");
    },
  });

  const disableMutation = useMutation({
    mutationFn: (id: string) => api.post(`/admin/extensions/${id}/disable`, {}),
    onSuccess: () => {
      toast.success("Extension disabled");
      queryClient.invalidateQueries({ queryKey: ["extensions"] });
    },
    onError: (err) => {
      toast.error(err instanceof ApiError ? err.message : "Failed to disable extension");
    },
  });

  const uninstallMutation = useMutation({
    mutationFn: ({ id, drop }: { id: string; drop: boolean }) =>
      apiRequest<void>(`/admin/extensions/${id}`, {
        method: "DELETE",
        body: JSON.stringify({ drop_tables: drop }),
      }),
    onSuccess: () => {
      toast.success("Extension uninstalled");
      setUninstallTarget(null);
      setDropTables(false);
      queryClient.invalidateQueries({ queryKey: ["extensions"] });
    },
    onError: (err) => {
      toast.error(err instanceof ApiError ? err.message : "Failed to uninstall extension");
    },
  });

  const extensions = listQuery.data ?? [];

  const enabledCount = extensions.filter((e) => e.enabled).length;
  const disabledCount = extensions.length - enabledCount;

  return (
    <div className="space-y-6">
      <div className="flex items-center justify-between">
        <h1 className="text-2xl font-bold">Extensions</h1>
        <div className="flex items-center gap-2">
          <Badge variant="default">{enabledCount} enabled</Badge>
          {disabledCount > 0 && (
            <Badge variant="outline">{disabledCount} disabled</Badge>
          )}
        </div>
      </div>

      <Card>
        <CardContent className="p-0">
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead>Extension</TableHead>
                <TableHead>Components</TableHead>
                <TableHead>Content Types</TableHead>
                <TableHead>Dependencies</TableHead>
                <TableHead>Status</TableHead>
                <TableHead className="text-right">Actions</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {listQuery.isLoading ? (
                <TableRow>
                  <TableCell colSpan={6} className="text-center py-8">
                    <div className="flex items-center justify-center gap-2">
                      <RefreshCw className="size-4 animate-spin" />
                      Loading...
                    </div>
                  </TableCell>
                </TableRow>
              ) : listQuery.isError ? (
                <TableRow>
                  <TableCell colSpan={6} className="text-center py-8">
                    <div className="flex flex-col items-center gap-2 text-destructive">
                      <AlertTriangle className="size-8" />
                      <p>Failed to load extensions</p>
                      <Button
                        variant="outline"
                        size="sm"
                        onClick={() => queryClient.invalidateQueries({ queryKey: ["extensions"] })}
                      >
                        Retry
                      </Button>
                    </div>
                  </TableCell>
                </TableRow>
              ) : extensions.length === 0 ? (
                <TableRow>
                  <TableCell colSpan={6} className="text-center py-8">
                    <div className="flex flex-col items-center gap-2 text-muted-foreground">
                      <Package className="size-8" />
                      <p>No extensions installed.</p>
                      <p className="text-xs">
                        Place extensions in the configured extension directory.
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
                                Contains {ext.content_types.length} content type(s)
                              </TooltipContent>
                            </Tooltip>
                          )}
                          {ext.has_plugin && (
                            <Tooltip>
                              <TooltipTrigger>
                                <Badge variant="outline" className="gap-1">
                                  <Puzzle className="size-3" />
                                  Plugin
                                </Badge>
                              </TooltipTrigger>
                              <TooltipContent>
                                Contains a plugin
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
                          <span className="text-xs text-muted-foreground">None</span>
                        )}
                      </TableCell>
                      <TableCell>
                        {ext.enabled ? (
                          <Badge variant="default">Enabled</Badge>
                        ) : (
                          <Badge variant="outline">Disabled</Badge>
                        )}
                      </TableCell>
                      <TableCell className="text-right">
                        <div className="flex items-center justify-end gap-1">
                          <Link href={`/admin/extensions/${encodeURIComponent(ext.id)}`}>
                            <Button variant="ghost" size="icon-sm" title="View details">
                              <ExternalLink className="size-4" />
                            </Button>
                          </Link>
                          {ext.enabled ? (
                            <Button
                              variant="ghost"
                              size="icon-sm"
                              title="Disable"
                              disabled={disableMutation.isPending}
                              onClick={() => disableMutation.mutate(ext.id)}
                            >
                              <PowerOff className="size-4" />
                            </Button>
                          ) : (
                            <Button
                              variant="ghost"
                              size="icon-sm"
                              title="Enable"
                              disabled={enableMutation.isPending}
                              onClick={() => enableMutation.mutate(ext.id)}
                            >
                              <Power className="size-4" />
                            </Button>
                          )}
                          <Button
                            variant="ghost"
                            size="icon-sm"
                            title="Uninstall"
                            onClick={() => {
                              setUninstallTarget(ext);
                              setDropTables(false);
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

      <Dialog
        open={!!uninstallTarget}
        onOpenChange={(open) => {
          if (!open) setUninstallTarget(null);
        }}
      >
        <DialogContent>
          <DialogHeader>
            <DialogTitle>Uninstall Extension</DialogTitle>
            <DialogDescription>
              Are you sure you want to uninstall{" "}
              <strong>{uninstallTarget?.name}</strong> (v
              {uninstallTarget?.version})? This will remove the extension files
              and database record.
            </DialogDescription>
          </DialogHeader>
          {uninstallTarget && uninstallTarget.content_types.length > 0 && (
            <div className="flex items-start gap-2 rounded-md border border-destructive/50 bg-destructive/5 p-3">
              <AlertTriangle className="size-4 mt-0.5 text-destructive shrink-0" />
              <div className="space-y-2">
                <p className="text-sm">
                  This extension contains content types:{" "}
                  <strong>{uninstallTarget.content_types.join(", ")}</strong>.
                  The associated database tables will remain unless you choose to
                  drop them.
                </p>
                <div className="flex items-center gap-2">
                  <Checkbox
                    id="drop-tables"
                    checked={dropTables}
                    onCheckedChange={(checked) => setDropTables(checked === true)}
                  />
                  <label htmlFor="drop-tables" className="text-sm font-medium">
                    Drop database tables (irreversible — all data will be lost)
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
              Cancel
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
              {uninstallMutation.isPending ? "Uninstalling..." : "Uninstall"}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </div>
  );
}
