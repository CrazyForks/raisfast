"use client";

import { useState } from "react";
import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import {
  Clock,
  Power,
  PowerOff,
  Trash2,
  Plus,
  ExternalLink,
  Trash,
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
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogTrigger,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { api, ApiError } from "@/lib/api";

interface CronSchedule {
  id: string;
  label: string;
  job_type: string;
  payload: string | null;
  cron_expr: string;
  enabled: boolean;
  last_run_at: string | null;
  next_run_at: string;
  plugin_id: string | null;
  created_at: string;
  updated_at: string;
}

interface PaginatedData<T> {
  items: T[];
  total: number;
  page: number;
  page_size: number;
}

function formatTime(iso: string | null): string {
  if (!iso) return "-";
  try {
    return new Date(iso).toLocaleString();
  } catch {
    return iso;
  }
}

function cronHuman(expr: string): string {
  const parts = expr.trim().split(/\s+/);
  if (parts.length < 6) return expr;
  const [, min, hour] = parts;
  if (min.includes("*/") && hour === "*") return `Every ${min.replace("*/", "")} min`;
  if (min === "0" && hour.includes("*/")) return `Every ${hour.replace("*/", "")}h`;
  if (min === "0" && hour === "0" && parts[3] === "*") return "Daily at midnight";
  if (min === "0" && hour === "0" && parts[3].includes("*/"))
    return `Every ${parts[3].replace("*/", "")} days`;
  return expr;
}

export default function CronsPage() {
  const queryClient = useQueryClient();
  const [dialogOpen, setDialogOpen] = useState(false);
  const [page, setPage] = useState(1);
  const pageSize = 20;
  const [form, setForm] = useState({
    label: "",
    job_type: "",
    payload: "",
    cron_expr: "",
    enabled: true,
  });

  const cronsQuery = useQuery({
    queryKey: ["crons", page],
    queryFn: () =>
      api.get<PaginatedData<CronSchedule>>(`/admin/crons?page=${page}&page_size=${pageSize}`),
  });

  const createMutation = useMutation({
    mutationFn: (body: Record<string, unknown>) =>
      api.post<CronSchedule>("/admin/crons", body),
    onSuccess: () => {
      toast.success("Schedule created");
      queryClient.invalidateQueries({ queryKey: ["crons"] });
      setDialogOpen(false);
      setForm({ label: "", job_type: "", payload: "", cron_expr: "", enabled: true });
    },
    onError: (err) => {
      toast.error(err instanceof ApiError ? err.message : "Failed to create");
    },
  });

  const toggleMutation = useMutation({
    mutationFn: ({ id, enabled }: { id: string; enabled: boolean }) =>
      api.post(`/admin/crons/${id}/toggle`, { enabled }),
    onSuccess: () => {
      toast.success("Schedule toggled");
      queryClient.invalidateQueries({ queryKey: ["crons"] });
    },
    onError: (err) => {
      toast.error(err instanceof ApiError ? err.message : "Failed to toggle");
    },
  });

  const deleteMutation = useMutation({
    mutationFn: (id: string) => api.delete(`/admin/crons/${id}`),
    onSuccess: () => {
      toast.success("Schedule deleted");
      queryClient.invalidateQueries({ queryKey: ["crons"] });
    },
    onError: (err) => {
      toast.error(err instanceof ApiError ? err.message : "Failed to delete");
    },
  });

  const crons = cronsQuery.data?.items ?? [];
  const totalPages = Math.ceil((cronsQuery.data?.total ?? 0) / pageSize);

  return (
    <div className="space-y-6">
      <div className="flex items-center justify-between">
        <h1 className="text-2xl font-bold">Cron Schedules</h1>
        <div className="flex items-center gap-2">
          <Badge variant="outline">{crons.length} schedule(s)</Badge>
          <Dialog open={dialogOpen} onOpenChange={setDialogOpen}>
            <DialogTrigger
              render={<Button size="sm" />}
            >
              <Plus className="size-4" />
              New Schedule
            </DialogTrigger>
            <DialogContent>
              <DialogHeader>
                <DialogTitle>Create Cron Schedule</DialogTitle>
              </DialogHeader>
              <div className="space-y-4 pt-2">
                <div className="space-y-2">
                  <Label htmlFor="label">Label</Label>
                  <Input
                    id="label"
                    value={form.label}
                    onChange={(e) =>
                      setForm((f) => ({ ...f, label: e.target.value }))
                    }
                    placeholder="e.g. Generate Sitemap"
                  />
                </div>
                <div className="space-y-2">
                  <Label htmlFor="job_type">Job Type</Label>
                  <Input
                    id="job_type"
                    value={form.job_type}
                    onChange={(e) =>
                      setForm((f) => ({ ...f, job_type: e.target.value }))
                    }
                    placeholder="e.g. generate_sitemap"
                  />
                </div>
                <div className="space-y-2">
                  <Label htmlFor="cron_expr">Cron Expression (7-segment)</Label>
                  <Input
                    id="cron_expr"
                    value={form.cron_expr}
                    onChange={(e) =>
                      setForm((f) => ({ ...f, cron_expr: e.target.value }))
                    }
                    placeholder="0 0 */6 * * * (every 6 hours)"
                  />
                  <p className="text-xs text-muted-foreground">
                    Format: sec min hour day month weekday
                  </p>
                </div>
                <div className="space-y-2">
                  <Label htmlFor="payload">Payload (JSON, optional)</Label>
                  <Input
                    id="payload"
                    value={form.payload}
                    onChange={(e) =>
                      setForm((f) => ({ ...f, payload: e.target.value }))
                    }
                    placeholder='{"key": "value"}'
                  />
                </div>
                <Button
                  className="w-full"
                  disabled={
                    createMutation.isPending ||
                    !form.label ||
                    !form.job_type ||
                    !form.cron_expr
                  }
                  onClick={() => {
                    createMutation.mutate({
                      label: form.label,
                      job_type: form.job_type,
                      cron_expr: form.cron_expr,
                      enabled: form.enabled,
                      ...(form.payload ? { payload: form.payload } : {}),
                    });
                  }}
                >
                  {createMutation.isPending ? "Creating..." : "Create"}
                </Button>
              </div>
            </DialogContent>
          </Dialog>
        </div>
      </div>

      <Card>
        <CardContent className="p-0">
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead>Label</TableHead>
                <TableHead>Job Type</TableHead>
                <TableHead>Schedule</TableHead>
                <TableHead>Status</TableHead>
                <TableHead>Last Run</TableHead>
                <TableHead>Next Run</TableHead>
                <TableHead className="text-right">Actions</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {cronsQuery.isLoading ? (
                <TableRow>
                  <TableCell colSpan={7} className="text-center py-8">
                    Loading...
                  </TableCell>
                </TableRow>
              ) : crons.length === 0 ? (
                <TableRow>
                  <TableCell colSpan={7} className="text-center py-8">
                    <div className="flex flex-col items-center gap-2 text-muted-foreground">
                      <Clock className="size-8" />
                      <p>No cron schedules.</p>
                      <p className="text-xs">
                        Create one above or enable CRON_SEED_ENABLED to seed
                        defaults.
                      </p>
                    </div>
                  </TableCell>
                </TableRow>
              ) : (
                crons.map((c) => (
                  <TableRow key={c.id}>
                    <TableCell>
                      <Link
                        href={`/admin/crons/${encodeURIComponent(c.id)}`}
                        className="block group"
                      >
                        <div className="font-medium group-hover:underline">
                          {c.label}
                        </div>
                        {c.plugin_id && (
                          <div className="text-xs text-muted-foreground">
                            Plugin: {c.plugin_id}
                          </div>
                        )}
                      </Link>
                    </TableCell>
                    <TableCell>
                      <Badge variant="ghost" className="font-mono text-xs">
                        {c.job_type}
                      </Badge>
                    </TableCell>
                    <TableCell>
                      <Tooltip>
                        <TooltipTrigger>
                          <span className="text-sm font-mono">
                            {cronHuman(c.cron_expr)}
                          </span>
                        </TooltipTrigger>
                        <TooltipContent>
                          <code className="text-xs">{c.cron_expr}</code>
                        </TooltipContent>
                      </Tooltip>
                    </TableCell>
                    <TableCell>
                      {c.enabled ? (
                        <Badge variant="default">Enabled</Badge>
                      ) : (
                        <Badge variant="outline">Disabled</Badge>
                      )}
                    </TableCell>
                    <TableCell className="text-sm text-muted-foreground">
                      {formatTime(c.last_run_at)}
                    </TableCell>
                    <TableCell className="text-sm">
                      {formatTime(c.next_run_at)}
                    </TableCell>
                    <TableCell className="text-right">
                      <div className="flex items-center justify-end gap-1">
                        <Link
                          href={`/admin/crons/${encodeURIComponent(c.id)}`}
                        >
                          <Button
                            variant="ghost"
                            size="icon-sm"
                            title="View details & logs"
                          >
                            <ExternalLink className="size-4" />
                          </Button>
                        </Link>
                        {c.enabled ? (
                          <Button
                            variant="ghost"
                            size="icon-sm"
                            title="Disable"
                            disabled={toggleMutation.isPending}
                            onClick={() =>
                              toggleMutation.mutate({
                                id: c.id,
                                enabled: false,
                              })
                            }
                          >
                            <PowerOff className="size-4" />
                          </Button>
                        ) : (
                          <Button
                            variant="ghost"
                            size="icon-sm"
                            title="Enable"
                            disabled={toggleMutation.isPending}
                            onClick={() =>
                              toggleMutation.mutate({
                                id: c.id,
                                enabled: true,
                              })
                            }
                          >
                            <Power className="size-4" />
                          </Button>
                        )}
                        <Button
                          variant="ghost"
                          size="icon-sm"
                          title="Delete"
                          disabled={deleteMutation.isPending}
                          onClick={() => {
                            if (
                              confirm(
                                `Delete schedule "${c.label}"? This cannot be undone.`,
                              )
                            ) {
                              deleteMutation.mutate(c.id);
                            }
                          }}
                        >
                          <Trash2 className="size-4 text-destructive" />
                        </Button>
                      </div>
                    </TableCell>
                  </TableRow>
                ))
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
            Previous
          </Button>
          <span className="text-sm text-muted-foreground">
            Page {page} of {totalPages}
          </span>
          <Button
            variant="outline"
            size="sm"
            disabled={page >= totalPages}
            onClick={() => setPage((p) => p + 1)}
          >
            Next
          </Button>
        </div>
      )}
    </div>
  );
}
