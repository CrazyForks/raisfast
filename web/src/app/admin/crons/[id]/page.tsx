"use client";

import { useRouter, useParams } from "next/navigation";
import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import Link from "next/link";
import {
  ArrowLeft,
  Power,
  PowerOff,
  Trash2,
  Clock,
  Activity,
  CheckCircle,
  XCircle,
  Loader2,
  Trash,
  FileText,
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

interface CronExecutionLog {
  id: string;
  schedule_id: string;
  job_type: string;
  label: string;
  status: string;
  duration_ms: number | null;
  error: string | null;
  started_at: string;
  finished_at: string | null;
}

function formatTime(iso: string | null): string {
  if (!iso) return "-";
  try {
    return new Date(iso).toLocaleString();
  } catch {
    return iso;
  }
}

function InfoRow({
  label,
  children,
}: {
  label: string;
  children: React.ReactNode;
}) {
  return (
    <div className="flex items-start justify-between py-2">
      <span className="text-sm text-muted-foreground">{label}</span>
      <div className="text-sm text-right">{children}</div>
    </div>
  );
}

function statusBadge(status: string) {
  switch (status) {
    case "success":
      return (
        <Badge variant="default" className="gap-1">
          <CheckCircle className="size-3" />
          Success
        </Badge>
      );
    case "failed":
      return (
        <Badge variant="destructive" className="gap-1">
          <XCircle className="size-3" />
          Failed
        </Badge>
      );
    case "running":
      return (
        <Badge variant="secondary" className="gap-1">
          <Loader2 className="size-3 animate-spin" />
          Running
        </Badge>
      );
    default:
      return <Badge variant="outline">{status}</Badge>;
  }
}

export default function CronDetailPage() {
  const router = useRouter();
  const params = useParams();
  const queryClient = useQueryClient();
  const id = decodeURIComponent(params.id as string);

  const scheduleQuery = useQuery({
    queryKey: ["cron", id],
    queryFn: () =>
      api.get<CronSchedule>(`/admin/crons/${encodeURIComponent(id)}`),
    retry: false,
  });

  const logsQuery = useQuery({
    queryKey: ["cron-logs", id],
    queryFn: () =>
      api.get<CronExecutionLog[]>(
        `/admin/crons/logs?schedule_id=${encodeURIComponent(id)}&limit=20`,
    ),
    enabled: !!scheduleQuery.data,
  });

  const toggleMutation = useMutation({
    mutationFn: (enabled: boolean) =>
      api.post(`/admin/crons/${encodeURIComponent(id)}/toggle`, { enabled }),
    onSuccess: () => {
      toast.success("Schedule toggled");
      queryClient.invalidateQueries({ queryKey: ["cron", id] });
      queryClient.invalidateQueries({ queryKey: ["crons"] });
    },
    onError: (err) => {
      toast.error(err instanceof ApiError ? err.message : "Failed to toggle");
    },
  });

  const deleteMutation = useMutation({
    mutationFn: () => api.delete(`/admin/crons/${encodeURIComponent(id)}`),
    onSuccess: () => {
      toast.success("Schedule deleted");
      queryClient.invalidateQueries({ queryKey: ["crons"] });
      router.push("/admin/crons");
    },
    onError: (err) => {
      toast.error(err instanceof ApiError ? err.message : "Failed to delete");
    },
  });

  const cleanupMutation = useMutation({
    mutationFn: () =>
      api.post<number>("/admin/crons/logs/cleanup", {}) as Promise<number>,
    onSuccess: (count: number) => {
      toast.success(`Cleaned up ${count} old log(s)`);
      queryClient.invalidateQueries({ queryKey: ["cron-logs", id] });
    },
    onError: (err) => {
      toast.error(
        err instanceof ApiError ? err.message : "Failed to cleanup",
      );
    },
  });

  if (scheduleQuery.isLoading) {
    return (
      <div className="space-y-6">
        <div className="flex items-center gap-4">
          <Link href="/admin/crons">
            <Button variant="outline" size="sm">
              &larr; Back
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
            </CardContent>
          </Card>
        </div>
      </div>
    );
  }

  if (scheduleQuery.error || !scheduleQuery.data) {
    return (
      <div className="space-y-6">
        <div className="flex items-center gap-4">
          <Link href="/admin/crons">
            <Button variant="outline" size="sm">
              &larr; Back
            </Button>
          </Link>
          <h1 className="text-2xl font-bold">Schedule Not Found</h1>
        </div>
        <Card>
          <CardContent className="pt-6">
            <div className="flex flex-col items-center gap-2 py-8 text-muted-foreground">
              <Clock className="size-8" />
              <p>
                {scheduleQuery.error instanceof ApiError
                  ? scheduleQuery.error.message
                  : "Schedule not found."}
              </p>
              <Link href="/admin/crons">
                <Button variant="outline" size="sm">
                  Back to Cron Schedules
                </Button>
              </Link>
            </div>
          </CardContent>
        </Card>
      </div>
    );
  }

  const schedule = scheduleQuery.data;
  const logs = logsQuery.data ?? [];

  return (
    <div className="space-y-6">
      <div className="flex items-center justify-between">
        <div className="flex items-center gap-4">
          <Link href="/admin/crons">
            <Button variant="outline" size="sm">
              &larr; Back
            </Button>
          </Link>
          <div>
            <h1 className="text-2xl font-bold">{schedule.label}</h1>
            <p className="text-sm text-muted-foreground font-mono">
              {schedule.job_type}
            </p>
          </div>
        </div>
        <div className="flex items-center gap-2">
          {schedule.enabled ? (
            <Button
              variant="outline"
              size="sm"
              disabled={toggleMutation.isPending}
              onClick={() => toggleMutation.mutate(false)}
            >
              <PowerOff className="size-4" />
              Disable
            </Button>
          ) : (
            <Button
              variant="outline"
              size="sm"
              disabled={toggleMutation.isPending}
              onClick={() => toggleMutation.mutate(true)}
            >
              <Power className="size-4" />
              Enable
            </Button>
          )}
          <Button
            variant="outline"
            size="sm"
            className="text-destructive hover:text-destructive"
            disabled={deleteMutation.isPending}
            onClick={() => {
              if (
                confirm(
                  `Delete schedule "${schedule.label}"? This cannot be undone.`,
                )
              ) {
                deleteMutation.mutate();
              }
            }}
          >
            <Trash2 className="size-4" />
            Delete
          </Button>
        </div>
      </div>

      <Card>
        <CardHeader>
          <CardTitle className="flex items-center gap-2">
            <FileText className="size-4" />
            Schedule Details
          </CardTitle>
        </CardHeader>
        <CardContent className="space-y-0">
          <InfoRow label="ID">
            <span className="font-mono text-xs">{schedule.id}</span>
          </InfoRow>
          <Separator />
          <InfoRow label="Label">{schedule.label}</InfoRow>
          <Separator />
          <InfoRow label="Job Type">
            <Badge variant="ghost" className="font-mono">
              {schedule.job_type}
            </Badge>
          </InfoRow>
          <Separator />
          <InfoRow label="Cron Expression">
            <Tooltip>
              <TooltipTrigger>
                <code className="text-sm bg-muted px-2 py-0.5 rounded">
                  {schedule.cron_expr}
                </code>
              </TooltipTrigger>
              <TooltipContent>
                7-segment: sec min hour day month weekday
              </TooltipContent>
            </Tooltip>
          </InfoRow>
          <Separator />
          <InfoRow label="Status">
            {schedule.enabled ? (
              <Badge variant="default">Enabled</Badge>
            ) : (
              <Badge variant="outline">Disabled</Badge>
            )}
          </InfoRow>
          <Separator />
          <InfoRow label="Payload">
            {schedule.payload ? (
              <code className="text-xs bg-muted px-2 py-0.5 rounded max-w-xs block truncate">
                {schedule.payload}
              </code>
            ) : (
              <span className="text-muted-foreground">None</span>
            )}
          </InfoRow>
          <Separator />
          <InfoRow label="Plugin">
            {schedule.plugin_id ? (
              <Badge variant="secondary">{schedule.plugin_id}</Badge>
            ) : (
              <span className="text-muted-foreground">Built-in</span>
            )}
          </InfoRow>
          <Separator />
          <InfoRow label="Last Run">{formatTime(schedule.last_run_at)}</InfoRow>
          <Separator />
          <InfoRow label="Next Run">{formatTime(schedule.next_run_at)}</InfoRow>
          <Separator />
          <InfoRow label="Created">{formatTime(schedule.created_at)}</InfoRow>
          <Separator />
          <InfoRow label="Updated">{formatTime(schedule.updated_at)}</InfoRow>
        </CardContent>
      </Card>

      <Card>
        <CardHeader className="flex flex-row items-center justify-between">
          <div>
            <CardTitle className="flex items-center gap-2">
              <Activity className="size-4" />
              Execution History
            </CardTitle>
            <CardDescription>
              Recent execution logs for this schedule
            </CardDescription>
          </div>
          <Button
            variant="outline"
            size="sm"
            disabled={cleanupMutation.isPending}
            onClick={() => cleanupMutation.mutate()}
          >
            <Trash className="size-4" />
            Cleanup Old
          </Button>
        </CardHeader>
        <CardContent className="p-0">
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead>Status</TableHead>
                <TableHead>Started</TableHead>
                <TableHead>Finished</TableHead>
                <TableHead>Duration</TableHead>
                <TableHead>Error</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {logsQuery.isLoading ? (
                <TableRow>
                  <TableCell colSpan={5} className="text-center py-8">
                    Loading...
                  </TableCell>
                </TableRow>
              ) : logs.length === 0 ? (
                <TableRow>
                  <TableCell colSpan={5} className="text-center py-8">
                    <div className="flex flex-col items-center gap-2 text-muted-foreground">
                      <Activity className="size-6" />
                      <p className="text-sm">No execution logs yet.</p>
                    </div>
                  </TableCell>
                </TableRow>
              ) : (
                logs.map((log) => (
                  <TableRow key={log.id}>
                    <TableCell>{statusBadge(log.status)}</TableCell>
                    <TableCell className="text-sm text-muted-foreground">
                      {formatTime(log.started_at)}
                    </TableCell>
                    <TableCell className="text-sm text-muted-foreground">
                      {formatTime(log.finished_at)}
                    </TableCell>
                    <TableCell className="text-sm">
                      {log.duration_ms != null ? `${log.duration_ms}ms` : "-"}
                    </TableCell>
                    <TableCell className="text-sm max-w-xs">
                      {log.error ? (
                        <Tooltip>
                          <TooltipTrigger>
                            <span className="text-destructive truncate block">
                              {log.error}
                            </span>
                          </TooltipTrigger>
                          <TooltipContent className="max-w-sm">
                            {log.error}
                          </TooltipContent>
                        </Tooltip>
                      ) : (
                        <span className="text-muted-foreground">-</span>
                      )}
                    </TableCell>
                  </TableRow>
                ))
              )}
            </TableBody>
          </Table>
        </CardContent>
      </Card>
    </div>
  );
}
