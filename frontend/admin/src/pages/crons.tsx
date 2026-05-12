
import { useState } from "react";
import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import { useT } from "@/lib/i18n";
import {
  Clock,
  Power,
  PowerOff,
  Trash2,
  Plus,
  ExternalLink,
  Trash,
  Pencil,
  Save,
  X,
  MoreVertical,
} from "lucide-react";
import { toast } from "sonner";
import Link from "@/lib/link";
import { useRouter } from "@/lib/navigation";

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
import { DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuTrigger } from "@/components/ui/dropdown-menu";
import { Label } from "@/components/ui/label";
import { client } from "@/lib/raisfast";
import { SDKError, type CronJob as CronJobSDK, type PaginatedData } from "@raisfast/sdk";

type CronJob = Omit<CronJobSDK, "id"> & { id: string };

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
  const { t } = useT();
  const router = useRouter();
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
  const [editCron, setEditCron] = useState<CronJob | null>(null);
  const [editLabel, setEditLabel] = useState("");
  const [editCronExpr, setEditCronExpr] = useState("");
  const [editPayload, setEditPayload] = useState("");

  const cronsQuery = useQuery({
    queryKey: ["crons", page],
    queryFn: async () => {
      const res = await client.admin.crons.list();
      const mapped: CronJob[] = res.map((c) => ({ ...c, id: String(c.id) }));
      return { items: mapped, total: mapped.length, page: 1, page_size: pageSize } as PaginatedData<CronJob>;
    },
  });

  const createMutation = useMutation({
    mutationFn: (body: Record<string, unknown>) =>
      client.send<CronJob>("/admin/crons", { method: "POST", body }),
    onSuccess: () => {
      toast.success(t("cron.scheduleCreated"));
      queryClient.invalidateQueries({ queryKey: ["crons"] });
      setDialogOpen(false);
      setForm({ label: "", job_type: "", payload: "", cron_expr: "", enabled: true });
    },
    onError: (err) => {
      toast.error(err instanceof SDKError ? err.message : t("cron.failedToCreate"));
    },
  });

  const toggleMutation = useMutation({
    mutationFn: ({ id, enabled }: { id: string; enabled: boolean }) =>
      client.admin.crons.toggle(id, enabled),
    onSuccess: () => {
      toast.success(t("cron.scheduleToggled"));
      queryClient.invalidateQueries({ queryKey: ["crons"] });
    },
    onError: (err) => {
      toast.error(err instanceof SDKError ? err.message : t("cron.failedToCreate"));
    },
  });

  const deleteMutation = useMutation({
    mutationFn: (id: string) => client.admin.crons.delete(id),
    onSuccess: () => {
      toast.success(t("cron.scheduleDeleted"));
      queryClient.invalidateQueries({ queryKey: ["crons"] });
    },
    onError: (err) => {
      toast.error(err instanceof SDKError ? err.message : t("cron.failedToDelete"));
    },
  });

  const updateMutation = useMutation({
    mutationFn: ({
      id,
      data,
    }: {
      id: string;
      data: { label?: string; cron_expr?: string; payload?: string };
    }) => client.admin.crons.update(id, data as never),
    onSuccess: () => {
      toast.success(t("cron.scheduleUpdated"));
      queryClient.invalidateQueries({ queryKey: ["crons"] });
      setEditCron(null);
    },
    onError: (err) => {
      toast.error(err instanceof SDKError ? err.message : "Failed to update");
    },
  });

  function startEdit(c: CronJob) {
    setEditCron(c);
    setEditLabel(c.label);
    setEditCronExpr(c.cron_expr);
    setEditPayload(c.payload ?? "");
  }

  function saveEdit() {
    if (!editCron) return;
    updateMutation.mutate({
      id: editCron.id,
      data: {
        label: editLabel,
        cron_expr: editCronExpr,
        ...(editPayload ? { payload: editPayload } : {}),
      },
    });
  }

  const crons = cronsQuery.data?.items ?? [];
  const totalPages = Math.ceil((cronsQuery.data?.total ?? 0) / pageSize);

  return (
    <div className="space-y-6">
      <div className="flex items-center justify-between">
        <h1 className="text-2xl font-bold">{t("cron.title")}</h1>
        <div className="flex items-center gap-2">
          <Badge variant="outline">{t("cron.schedules", { count: crons.length })}</Badge>
          <Dialog open={dialogOpen} onOpenChange={setDialogOpen}>
            <DialogTrigger
              render={<Button size="sm" />}
            >
              <Plus className="size-4" />
              {t("cron.newSchedule")}
            </DialogTrigger>
            <DialogContent>
              <DialogHeader>
                <DialogTitle>{t("cron.createSchedule")}</DialogTitle>
              </DialogHeader>
              <div className="space-y-4 pt-2">
                <div className="space-y-2">
                  <Label htmlFor="label">{t("cron.labelField")}</Label>
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
                  <Label htmlFor="job_type">{t("cron.jobType")}</Label>
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
                  <Label htmlFor="cron_expr">{t("cron.cronExpression")}</Label>
                  <Input
                    id="cron_expr"
                    value={form.cron_expr}
                    onChange={(e) =>
                      setForm((f) => ({ ...f, cron_expr: e.target.value }))
                    }
                    placeholder="0 0 */6 * * * (every 6 hours)"
                  />
                  <p className="text-xs text-muted-foreground">
                    {t("cron.format")}
                  </p>
                </div>
                <div className="space-y-2">
                  <Label htmlFor="payload">{t("cron.payload")}</Label>
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
                  {createMutation.isPending ? t("common.creating") : t("common.create")}
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
                <TableHead>{t("cron.labelField")}</TableHead>
                <TableHead>{t("cron.jobType")}</TableHead>
                <TableHead>{t("cron.schedule")}</TableHead>
                <TableHead>{t("common.status")}</TableHead>
                <TableHead>{t("cron.lastRun")}</TableHead>
                <TableHead>{t("cron.nextRun")}</TableHead>
                <TableHead className="text-right">{t("common.actions")}</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {cronsQuery.isLoading ? (
                <TableRow>
                  <TableCell colSpan={7} className="text-center py-8">
                    {t("common.loading")}
                  </TableCell>
                </TableRow>
              ) : crons.length === 0 ? (
                <TableRow>
                  <TableCell colSpan={7} className="text-center py-8">
                    <div className="flex flex-col items-center gap-2 text-muted-foreground">
                      <Clock className="size-8" />
                      <p>{t("cron.noCronJobs")}</p>
                      <p className="text-xs">
                        {t("cron.seedHint")}
                      </p>
                    </div>
                  </TableCell>
                </TableRow>
              ) : (
                crons.map((c) => (
                  <TableRow key={c.id}>
                    <TableCell>
                      {editCron?.id === c.id ? (
                        <Input
                          value={editLabel}
                          onChange={(e) => setEditLabel(e.target.value)}
                          className="h-8 w-40"
                        />
                      ) : (
                        <Link
                          href={`/admin/crons/${encodeURIComponent(c.id)}`}
                          className="block group"
                        >
                          <div className="font-medium group-hover:underline">
                            {c.label}
                          </div>
                          {c.plugin_id && (
                            <div className="text-xs text-muted-foreground">
                              {t("cron.pluginCol")}: {c.plugin_id}
                            </div>
                          )}
                        </Link>
                      )}
                    </TableCell>
                    <TableCell>
                      <Badge variant="ghost" className="font-mono text-xs">
                        {c.job_type}
                      </Badge>
                    </TableCell>
                    <TableCell>
                      {editCron?.id === c.id ? (
                        <Input
                          value={editCronExpr}
                          onChange={(e) => setEditCronExpr(e.target.value)}
                          className="h-8 w-40 font-mono text-xs"
                        />
                      ) : (
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
                      )}
                    </TableCell>
                    <TableCell>
                      {c.enabled ? (
                        <Badge variant="default">{t("common.enabled")}</Badge>
                      ) : (
                        <Badge variant="outline">{t("common.disabled")}</Badge>
                      )}
                    </TableCell>
                    <TableCell className="text-sm text-muted-foreground">
                      {formatTime(c.last_run_at)}
                    </TableCell>
                    <TableCell className="text-sm">
                      {formatTime(c.next_run_at)}
                    </TableCell>
                    <TableCell className="text-right">
                      {editCron?.id === c.id ? (
                        <div className="flex items-center justify-end gap-1">
                          <Button
                            variant="ghost"
                            size="icon-sm"
                            onClick={saveEdit}
                            disabled={updateMutation.isPending}
                          >
                            <Save className="size-4" />
                          </Button>
                          <Button
                            variant="ghost"
                            size="icon-sm"
                            onClick={() => setEditCron(null)}
                          >
                            <X className="size-4" />
                          </Button>
                        </div>
                      ) : (
                        <div className="flex items-center justify-end">
                          <DropdownMenu>
                            <DropdownMenuTrigger className="inline-flex items-center justify-center rounded-md p-1 hover:bg-muted transition-colors">
                              <MoreVertical className="size-4" />
                            </DropdownMenuTrigger>
                            <DropdownMenuContent align="end">
                              <DropdownMenuItem onClick={() => startEdit(c)}>
                                <Pencil className="size-4 mr-2" />
                                Edit
                              </DropdownMenuItem>
                              <DropdownMenuItem onClick={() => router.push(`/admin/crons/${encodeURIComponent(c.id)}`)}>
                                <ExternalLink className="size-4 mr-2" />
                                View details & logs
                              </DropdownMenuItem>
                              {c.enabled ? (
                                <DropdownMenuItem
                                  disabled={toggleMutation.isPending}
                                  onClick={() =>
                                    toggleMutation.mutate({ id: c.id, enabled: false })
                                  }
                                >
                                  <PowerOff className="size-4 mr-2" />
                                  Disable
                                </DropdownMenuItem>
                              ) : (
                                <DropdownMenuItem
                                  disabled={toggleMutation.isPending}
                                  onClick={() =>
                                    toggleMutation.mutate({ id: c.id, enabled: true })
                                  }
                                >
                                  <Power className="size-4 mr-2" />
                                  Enable
                                </DropdownMenuItem>
                              )}
                              <DropdownMenuItem
                                disabled={deleteMutation.isPending}
                                onClick={() => {
                                  if (
                                    confirm(
                                      t("cron.confirmDeleteSchedule", { name: c.label }),
                                    )
                                  ) {
                                    deleteMutation.mutate(c.id);
                                  }
                                }}
                              >
                                <Trash2 className="size-4 mr-2 text-destructive" />
                                Delete
                              </DropdownMenuItem>
                            </DropdownMenuContent>
                          </DropdownMenu>
                        </div>
                      )}
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
