"use client";

import { useState } from "react";
import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import { Eye, GitBranch, XCircle, ArrowRight, ChevronDown, ChevronUp } from "lucide-react";
import { toast } from "sonner";
import Link from "next/link";

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
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { api, ApiError } from "@/lib/api";
import { useT } from "@/lib/i18n";

interface WorkflowInstance {
  id: string;
  definition_id: string;
  status: string;
  current_step: string | null;
  context: string;
  triggered_by: string | null;
  started_at: string;
  completed_at: string | null;
  updated_at: string;
}

interface StepLog {
  id: string;
  instance_id: string;
  step_id: string;
  step_name: string;
  status: string;
  input: string | null;
  output: string | null;
  error: string | null;
  started_at: string;
  completed_at: string | null;
}

interface PaginatedData<T> {
  items: T[];
  total: number;
  page: number;
  page_size: number;
}

function StatusBadge({ status }: { status: string }) {
  const variant =
    status === "completed"
      ? "default"
      : status === "running"
        ? "secondary"
        : status === "failed"
          ? "destructive"
          : "outline";
  return <Badge variant={variant}>{status}</Badge>;
}

function InstanceDetail({ instance }: { instance: WorkflowInstance }) {
  const { t } = useT();
  const [expanded, setExpanded] = useState(false);

  const logsQuery = useQuery({
    queryKey: ["workflow-step-logs", instance.id],
    queryFn: () => api.get<StepLog[]>(`/admin/workflows/instances/${instance.id}/logs`),
    enabled: expanded,
  });

  const queryClient = useQueryClient();

  const cancelMutation = useMutation({
    mutationFn: () => api.post(`/admin/workflows/instances/${instance.id}/cancel`, {}),
    onSuccess: () => {
      toast.success(t("workflows.instances.cancelled"));
      queryClient.invalidateQueries({ queryKey: ["workflow-instances"] });
    },
    onError: (err) => {
      if (err instanceof ApiError) toast.error(err.message);
    },
  });

  const logs = logsQuery.data ?? [];

  return (
    <div className="space-y-2">
      <div className="flex items-center gap-2">
        <Button variant="ghost" size="sm" onClick={() => setExpanded(!expanded)}>
          {expanded ? <ChevronUp className="size-4" /> : <ChevronDown className="size-4" />}
          {expanded ? t("workflows.instances.hideLogs") : t("workflows.instances.showLogs")}
        </Button>
        {instance.status === "running" && (
          <Button
            variant="ghost"
            size="sm"
            onClick={() => cancelMutation.mutate()}
            disabled={cancelMutation.isPending}
          >
            <XCircle className="size-4" />
            {t("common.cancel")}
          </Button>
        )}
        {instance.current_step && (
          <span className="text-xs text-muted-foreground">
            {t("workflows.instances.currentStep")} <code className="bg-muted px-1 rounded">{instance.current_step}</code>
          </span>
        )}
      </div>

      {expanded && (
        <Card>
          <CardContent className="p-3 space-y-3">
            <div>
              <p className="text-xs font-medium text-muted-foreground mb-1">{t("workflows.instances.context")}</p>
              <pre className="text-xs bg-muted p-2 rounded overflow-x-auto">
                {(() => {
                  try {
                    return JSON.stringify(JSON.parse(instance.context), null, 2);
                  } catch {
                    return instance.context;
                  }
                })()}
              </pre>
            </div>

            <div>
              <p className="text-xs font-medium text-muted-foreground mb-1">
                {t("workflows.instances.stepLogs", { count: logs.length })}
              </p>
              {logsQuery.isLoading ? (
                <p className="text-xs text-muted-foreground">{t("common.loading")}</p>
              ) : logs.length === 0 ? (
                <p className="text-xs text-muted-foreground">{t("workflows.instances.noStepLogs")}</p>
              ) : (
                <div className="space-y-2">
                  {logs.map((log) => (
                    <div key={log.id} className="flex items-start gap-2 text-xs">
                      <StatusBadge status={log.status} />
                      <div className="flex-1">
                        <div className="flex items-center gap-1">
                          <span className="font-medium">{log.step_name}</span>
                          <code className="text-muted-foreground">{log.step_id}</code>
                        </div>
                        {log.error && (
                          <p className="text-red-500 mt-0.5">{log.error}</p>
                        )}
                        <p className="text-muted-foreground mt-0.5">
                          {new Date(log.started_at).toLocaleString()}
                          {log.completed_at && (
                            <>
                              {" → "}
                              {new Date(log.completed_at).toLocaleString()}
                            </>
                          )}
                        </p>
                      </div>
                    </div>
                  ))}
                </div>
              )}
            </div>
          </CardContent>
        </Card>
      )}
    </div>
  );
}

export default function WorkflowInstancesPage() {
  const { t } = useT();
  const [page, setPage] = useState(1);
  const [statusFilter, setStatusFilter] = useState<string>("all");
  const pageSize = 20;

  const instancesQuery = useQuery({
    queryKey: ["workflow-instances", page, statusFilter],
    queryFn: () => {
      const params = new URLSearchParams({ page: String(page), page_size: String(pageSize) });
      if (statusFilter !== "all") params.set("status", statusFilter);
      return api.get<PaginatedData<WorkflowInstance>>(
        `/admin/workflows/instances?${params.toString()}`,
      );
    },
  });

  const instances = instancesQuery.data?.items ?? [];
  const totalPages = Math.ceil((instancesQuery.data?.total ?? 0) / pageSize);

  return (
    <div className="space-y-6">
      <div className="flex items-center justify-between">
        <div className="flex items-center gap-2">
          <GitBranch className="size-6" />
          <h1 className="text-2xl font-bold">{t("workflows.instances.title")}</h1>
        </div>
        <Link href="/admin/workflows">
          <Button variant="outline" size="sm">
            <ArrowRight className="size-4 rotate-180" />
            {t("workflows.instances.definitions")}
          </Button>
        </Link>
      </div>

      <div className="flex items-center gap-2">
        <Select value={statusFilter} onValueChange={(v) => { setStatusFilter(v ?? "all"); setPage(1); }}>
          <SelectTrigger className="w-40">
            <SelectValue placeholder="Filter status" />
          </SelectTrigger>
          <SelectContent>
            <SelectItem value="all">{t("common.all")}</SelectItem>
            <SelectItem value="running">Running</SelectItem>
            <SelectItem value="completed">Completed</SelectItem>
            <SelectItem value="failed">Failed</SelectItem>
            <SelectItem value="cancelled">Cancelled</SelectItem>
          </SelectContent>
        </Select>
        <span className="text-sm text-muted-foreground">
          {t("workflows.instances.instanceCount", { count: instancesQuery.data?.total ?? 0 })}
        </span>
      </div>

      <div className="space-y-4">
        {instancesQuery.isLoading ? (
          <div className="text-center py-8 text-muted-foreground">{t("common.loading")}</div>
        ) : instances.length === 0 ? (
          <div className="text-center py-8 text-muted-foreground">
            {t("workflows.instances.noInstances")}
          </div>
        ) : (
          instances.map((inst) => (
            <Card key={inst.id}>
              <CardHeader className="pb-2">
                <div className="flex items-center justify-between">
                  <CardTitle className="text-sm font-medium">
                    <code className="bg-muted px-1.5 py-0.5 rounded text-xs">{inst.id}</code>
                    <span className="mx-2 text-muted-foreground">→</span>
                    <code className="text-xs text-muted-foreground">{inst.definition_id}</code>
                  </CardTitle>
                  <div className="flex items-center gap-2">
                    <StatusBadge status={inst.status} />
                    <span className="text-xs text-muted-foreground">
                      {new Date(inst.started_at).toLocaleString()}
                    </span>
                  </div>
                </div>
              </CardHeader>
              <CardContent className="pt-0">
                <InstanceDetail instance={inst} />
              </CardContent>
            </Card>
          ))
        )}
      </div>

      {totalPages > 1 && (
        <div className="flex items-center justify-center gap-2">
          <Button variant="outline" size="sm" disabled={page <= 1} onClick={() => setPage((p) => p - 1)}>
            {t("common.previous")}
          </Button>
          <span className="text-sm text-muted-foreground">
            Page {t("common.pageOf", { page, total: totalPages })}
          </span>
          <Button variant="outline" size="sm" disabled={page >= totalPages} onClick={() => setPage((p) => p + 1)}>
            {t("common.next")}
          </Button>
        </div>
      )}
    </div>
  );
}
