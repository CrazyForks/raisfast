"use client";

import { use, useState } from "react";
import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import Link from "next/link";
import {
  ArrowLeft,
  Package,
  Power,
  PowerOff,
  Trash2,
  Layers,
  Puzzle,
  ExternalLink,
  AlertTriangle,
  RefreshCw,
  Calendar,
  Tag as TagIcon,
  Pencil,
  List,
} from "lucide-react";
import { toast } from "sonner";

import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
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
import { Separator } from "@/components/ui/separator";
import { api, apiRequest, ApiError } from "@/lib/api";

interface ExtensionDetail {
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

export default function ExtensionDetailPage({
  params,
}: {
  params: Promise<{ id: string }>;
}) {
  const { id } = use(params);
  const queryClient = useQueryClient();
  const [showUninstall, setShowUninstall] = useState(false);
  const [dropTables, setDropTables] = useState(false);

  const detailQuery = useQuery({
    queryKey: ["extension", id],
    queryFn: () => api.get<ExtensionDetail>(`/admin/extensions/${id}`),
    retry: false,
  });

  const enableMutation = useMutation({
    mutationFn: () => api.post(`/admin/extensions/${id}/enable`, {}),
    onSuccess: () => {
      toast.success("Extension enabled");
      queryClient.invalidateQueries({ queryKey: ["extension", id] });
      queryClient.invalidateQueries({ queryKey: ["extensions"] });
    },
    onError: (err) => {
      toast.error(err instanceof ApiError ? err.message : "Failed to enable");
    },
  });

  const disableMutation = useMutation({
    mutationFn: () => api.post(`/admin/extensions/${id}/disable`, {}),
    onSuccess: () => {
      toast.success("Extension disabled");
      queryClient.invalidateQueries({ queryKey: ["extension", id] });
      queryClient.invalidateQueries({ queryKey: ["extensions"] });
    },
    onError: (err) => {
      toast.error(err instanceof ApiError ? err.message : "Failed to disable");
    },
  });

  const uninstallMutation = useMutation({
    mutationFn: (drop: boolean) =>
      apiRequest<void>(`/admin/extensions/${id}`, {
        method: "DELETE",
        body: JSON.stringify({ drop_tables: drop }),
      }),
    onSuccess: () => {
      toast.success("Extension uninstalled");
      queryClient.invalidateQueries({ queryKey: ["extensions"] });
      window.location.href = "/admin/extensions";
    },
    onError: (err) => {
      toast.error(
        err instanceof ApiError ? err.message : "Failed to uninstall",
      );
    },
  });

  if (detailQuery.isLoading) {
    return (
      <div className="flex items-center justify-center py-16">
        <RefreshCw className="size-6 animate-spin text-muted-foreground" />
      </div>
    );
  }

  if (detailQuery.isError || !detailQuery.data) {
    return (
      <div className="space-y-6">
        <Link
          href="/admin/extensions"
          className="inline-flex items-center gap-1.5 text-sm text-muted-foreground hover:text-foreground"
        >
          <ArrowLeft className="size-4" />
          Back to Extensions
        </Link>
        <div className="flex flex-col items-center gap-2 py-16 text-destructive">
          <AlertTriangle className="size-8" />
          <p>Extension not found</p>
          <p className="text-sm text-muted-foreground">
            The extension &quot;{id}&quot; does not exist or has been removed.
          </p>
        </div>
      </div>
    );
  }

  const ext = detailQuery.data;
  const depKeys = Object.keys(ext.dependencies);

  return (
    <div className="space-y-6">
      <div className="flex items-center gap-2">
        <Link
          href="/admin/extensions"
          className="inline-flex items-center gap-1.5 text-sm text-muted-foreground hover:text-foreground"
        >
          <ArrowLeft className="size-4" />
          Extensions
        </Link>
      </div>

      <div className="flex items-start justify-between">
        <div className="space-y-1">
          <div className="flex items-center gap-2">
            <h1 className="text-2xl font-bold">{ext.name}</h1>
            {ext.enabled ? (
              <Badge variant="default">Enabled</Badge>
            ) : (
              <Badge variant="outline">Disabled</Badge>
            )}
          </div>
          <div className="flex items-center gap-3 text-sm text-muted-foreground">
            <span>{ext.id}</span>
            <span>v{ext.version}</span>
            {ext.installed_at && (
              <span className="inline-flex items-center gap-1">
                <Calendar className="size-3" />
                Installed {new Date(ext.installed_at).toLocaleDateString()}
              </span>
            )}
          </div>
          {ext.description && (
            <p className="text-sm text-muted-foreground max-w-xl">
              {ext.description}
            </p>
          )}
        </div>
        <div className="flex items-center gap-2">
          {ext.enabled ? (
            <Button
              variant="outline"
              size="sm"
              disabled={disableMutation.isPending}
              onClick={() => disableMutation.mutate()}
            >
              <PowerOff className="size-4" />
              Disable
            </Button>
          ) : (
            <Button
              size="sm"
              disabled={enableMutation.isPending}
              onClick={() => enableMutation.mutate()}
            >
              <Power className="size-4" />
              Enable
            </Button>
          )}
          <Button
            variant="outline"
            size="sm"
            className="text-destructive hover:text-destructive"
            onClick={() => {
              setShowUninstall(true);
              setDropTables(false);
            }}
          >
            <Trash2 className="size-4" />
            Uninstall
          </Button>
        </div>
      </div>

      <div className="grid gap-6 md:grid-cols-3">
        <Card>
          <CardHeader className="pb-2">
            <CardTitle className="text-sm font-medium text-muted-foreground">
              Components
            </CardTitle>
          </CardHeader>
          <CardContent className="flex items-center gap-3">
            {ext.has_content_types ? (
              <Tooltip>
                <TooltipTrigger>
                  <Badge variant="secondary" className="gap-1.5 py-1 px-2.5">
                    <Layers className="size-3.5" />
                    Content Types
                  </Badge>
                </TooltipTrigger>
                <TooltipContent>
                  {ext.content_types.length} content type(s)
                </TooltipContent>
              </Tooltip>
            ) : (
              <Badge variant="ghost" className="opacity-50">
                No Content Types
              </Badge>
            )}
            {ext.has_plugin ? (
              <Badge variant="outline" className="gap-1.5 py-1 px-2.5">
                <Puzzle className="size-3.5" />
                Plugin
              </Badge>
            ) : (
              <Badge variant="ghost" className="opacity-50">
                No Plugin
              </Badge>
            )}
          </CardContent>
        </Card>

        <Card>
          <CardHeader className="pb-2">
            <CardTitle className="text-sm font-medium text-muted-foreground">
              Dependencies
            </CardTitle>
          </CardHeader>
          <CardContent>
            {depKeys.length > 0 ? (
              <div className="flex flex-wrap gap-1.5">
                {depKeys.map((dep) => (
                  <Tooltip key={dep}>
                    <TooltipTrigger>
                      <Link href={`/admin/extensions/${dep}`}>
                        <Badge
                          variant="secondary"
                          className="gap-1 cursor-pointer hover:bg-secondary/80"
                        >
                          <Package className="size-3" />
                          {dep}
                        </Badge>
                      </Link>
                    </TooltipTrigger>
                    <TooltipContent>
                      {dep} {ext.dependencies[dep]}
                    </TooltipContent>
                  </Tooltip>
                ))}
              </div>
            ) : (
              <span className="text-sm text-muted-foreground">
                No dependencies
              </span>
            )}
          </CardContent>
        </Card>

        <Card>
          <CardHeader className="pb-2">
            <CardTitle className="text-sm font-medium text-muted-foreground">
              Installation
            </CardTitle>
          </CardHeader>
          <CardContent>
            {ext.installed ? (
              <div className="space-y-1 text-sm">
                <div className="flex items-center gap-1.5">
                  <Badge variant="default" className="size-1.5 rounded-full p-0" />
                  Installed
                </div>
                {ext.installed_at && (
                  <p className="text-muted-foreground">
                    {new Date(ext.installed_at).toLocaleString()}
                  </p>
                )}
              </div>
            ) : (
              <span className="text-sm text-muted-foreground">
                Not installed (file-only)
              </span>
            )}
          </CardContent>
        </Card>
      </div>

      <Separator />

      {ext.has_content_types && (
        <Card>
          <CardHeader>
            <div className="flex items-center justify-between">
              <CardTitle className="flex items-center gap-2">
                <Layers className="size-5" />
                Content Types
              </CardTitle>
              <Badge variant="outline">{ext.content_types.length}</Badge>
            </div>
          </CardHeader>
          <CardContent className="p-0">
            <Table>
              <TableHeader>
                <TableRow>
                  <TableHead>Name</TableHead>
                  <TableHead className="text-right">Actions</TableHead>
                </TableRow>
              </TableHeader>
              <TableBody>
                {ext.content_types.map((ct) => (
                  <TableRow key={ct}>
                    <TableCell>
                      <div className="flex items-center gap-2">
                        <TagIcon className="size-4 text-muted-foreground" />
                        <span className="font-medium">{ct}</span>
                      </div>
                    </TableCell>
                    <TableCell className="text-right">
                      <div className="flex items-center justify-end gap-1">
                        <Link href={`/admin/content-types/${ct}`}>
                          <Button variant="ghost" size="sm" className="gap-1.5">
                            <List className="size-3.5" />
                            Data
                          </Button>
                        </Link>
                        <Link href={`/admin/content-types/builder?edit=${ct}`}>
                          <Button variant="ghost" size="sm" className="gap-1.5">
                            <Pencil className="size-3.5" />
                            Schema
                          </Button>
                        </Link>
                      </div>
                    </TableCell>
                  </TableRow>
                ))}
              </TableBody>
            </Table>
          </CardContent>
        </Card>
      )}

      {ext.has_plugin && (
        <Card>
          <CardHeader>
            <CardTitle className="flex items-center gap-2">
              <Puzzle className="size-5" />
              Plugin
            </CardTitle>
          </CardHeader>
          <CardContent>
            <div className="flex items-center justify-between">
              <div>
                <p className="text-sm text-muted-foreground">
                  This extension contains a plugin with runtime logic (hooks,
                  cron tasks, custom routes).
                </p>
              </div>
              <Link href={`/admin/plugins/${ext.id}`}>
                <Button variant="outline" size="sm" className="gap-1.5">
                  <ExternalLink className="size-3.5" />
                  View Runtime Details
                </Button>
              </Link>
            </div>
          </CardContent>
        </Card>
      )}

      <Dialog open={showUninstall} onOpenChange={setShowUninstall}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>Uninstall Extension</DialogTitle>
            <DialogDescription>
              Are you sure you want to uninstall <strong>{ext.name}</strong> (v
              {ext.version})? This will remove the extension files and database
              record.
            </DialogDescription>
          </DialogHeader>
          {ext.content_types.length > 0 && (
            <div className="flex items-start gap-2 rounded-md border border-destructive/50 bg-destructive/5 p-3">
              <AlertTriangle className="size-4 mt-0.5 text-destructive shrink-0" />
              <div className="space-y-2">
                <p className="text-sm">
                  This extension contains content types:{" "}
                  <strong>{ext.content_types.join(", ")}</strong>. The associated
                  database tables will remain unless you choose to drop them.
                </p>
                <div className="flex items-center gap-2">
                  <Checkbox
                    id="drop-tables-detail"
                    checked={dropTables}
                    onCheckedChange={(c) => setDropTables(c === true)}
                  />
                  <label
                    htmlFor="drop-tables-detail"
                    className="text-sm font-medium"
                  >
                    Drop database tables (irreversible)
                  </label>
                </div>
              </div>
            </div>
          )}
          <DialogFooter>
            <Button
              variant="outline"
              onClick={() => setShowUninstall(false)}
              disabled={uninstallMutation.isPending}
            >
              Cancel
            </Button>
            <Button
              variant="destructive"
              disabled={uninstallMutation.isPending}
              onClick={() => uninstallMutation.mutate(dropTables)}
            >
              {uninstallMutation.isPending ? "Uninstalling..." : "Uninstall"}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </div>
  );
}
