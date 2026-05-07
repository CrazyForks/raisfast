
import { useState } from "react";
import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import {
  CheckCircle,
  XCircle,
  Trash2,
  MessageSquare,
  Filter,
  CheckSquare,
  MoreVertical,
} from "lucide-react";
import { toast } from "sonner";
import Link from "@/lib/link";

import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { Card, CardContent } from "@/components/ui/card";
import { Checkbox } from "@/components/ui/checkbox";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { client } from "@/lib/raisfast";
import { SDKError } from "@raisfast/sdk";
import { useAuthStore } from "@/stores/auth";
import { useT } from "@/lib/i18n";

interface AdminComment {
  id: string;
  post_id: string;
  post_title: string;
  created_by: string | null;
  updated_by: string | null;
  nickname: string | null;
  email: string | null;
  content: string;
  parent_id: string | null;
  status: string;
  created_at: string;
}

interface PaginatedData<T> {
  items: T[];
  total: number;
  page: number;
  page_size: number;
}

function statusBadgeVariant(status: string) {
  switch (status) {
    case "approved":
      return "default" as const;
    case "rejected":
    case "spam":
      return "destructive" as const;
    default:
      return "secondary" as const;
  }
}

const STATUS_OPTIONS = [
  { value: "pending", label: "Pending" },
  { value: "approved", label: "Approved" },
  { value: "rejected", label: "Rejected" },
];

export default function CommentsPage() {
  const { t } = useT();
  const { isAdmin } = useAuthStore();
  const queryClient = useQueryClient();
  const [page, setPage] = useState(1);
  const [statusFilter, setStatusFilter] = useState<string>("all");
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const pageSize = 20;

  const commentsQuery = useQuery({
    queryKey: ["admin-comments", page],
    queryFn: () => client.comments.listAll(page, pageSize),
    enabled: isAdmin(),
  });

  const statusMutation = useMutation({
    mutationFn: ({ id, status }: { id: string; status: string }) =>
      client.comments.updateStatus(id, status),
    onSuccess: () => {
      toast.success(t("comments.commentStatusUpdated"));
      queryClient.invalidateQueries({ queryKey: ["admin-comments"] });
    },
    onError: (err) => {
      if (err instanceof SDKError) {
        toast.error(err.message);
      } else {
        toast.error(t("comments.failedToUpdateStatus"));
      }
    },
  });

  const deleteMutation = useMutation({
    mutationFn: (id: string) => client.comments.delete(id),
    onSuccess: () => {
      toast.success(t("comments.commentDeleted"));
      queryClient.invalidateQueries({ queryKey: ["admin-comments"] });
      setSelected(new Set());
    },
    onError: (err) => {
      if (err instanceof SDKError) {
        toast.error(err.message);
      } else {
        toast.error(t("comments.failedToDelete"));
      }
    },
  });

  const bulkStatusMutation = useMutation({
    mutationFn: ({ ids, status }: { ids: string[]; status: string }) =>
      Promise.all(ids.map((id) => client.comments.updateStatus(id, status))),
    onSuccess: (_data, vars) => {
      toast.success(t("comments.bulkActionDone", { count: vars.ids.length, status: vars.status }));
      queryClient.invalidateQueries({ queryKey: ["admin-comments"] });
      setSelected(new Set());
    },
    onError: (err) => {
      if (err instanceof SDKError) {
        toast.error(err.message);
      } else {
        toast.error(t("comments.bulkActionFailed"));
      }
    },
  });

  const bulkDeleteMutation = useMutation({
    mutationFn: (ids: string[]) =>
      Promise.all(ids.map((id) => client.comments.delete(id))),
    onSuccess: (_data, ids) => {
      toast.success(t("comments.bulkDeleted", { count: ids.length }));
      queryClient.invalidateQueries({ queryKey: ["admin-comments"] });
      setSelected(new Set());
    },
    onError: (err) => {
      if (err instanceof SDKError) {
        toast.error(err.message);
      } else {
        toast.error(t("comments.bulkDeleteFailed"));
      }
    },
  });

  function handleDelete(id: string) {
    if (confirm(t("comments.confirmDelete"))) {
      deleteMutation.mutate(id);
    }
  }

  function toggleSelect(id: string) {
    setSelected((prev) => {
      const next = new Set(prev);
      if (next.has(id)) {
        next.delete(id);
      } else {
        next.add(id);
      }
      return next;
    });
  }

  function toggleSelectAll(ids: string[]) {
    setSelected((prev) => {
      const allSelected = ids.every((id) => prev.has(id));
      if (allSelected) {
        return new Set();
      }
      return new Set(ids);
    });
  }

  function handleBulkStatus(status: string) {
    if (selected.size === 0) return;
    bulkStatusMutation.mutate({ ids: Array.from(selected), status });
  }

  function handleBulkDelete() {
    if (selected.size === 0) return;
    if (confirm(t("comments.confirmBulkDelete", { count: selected.size }))) {
      bulkDeleteMutation.mutate(Array.from(selected));
    }
  }

  if (!isAdmin()) {
    return (
      <div className="flex min-h-[50vh] items-center justify-center">
        <div className="text-center space-y-4">
          <MessageSquare className="size-12 mx-auto text-muted-foreground" />
          <h2 className="text-xl font-semibold">{t("common.adminOnly")}</h2>
          <p className="text-muted-foreground">
            {t("common.adminOnlyMsg")}
          </p>
          <Link href="/dashboard">
            <Button variant="outline">{t("common.backToDashboard")}</Button>
          </Link>
        </div>
      </div>
    );
  }

  const allComments = commentsQuery.data?.items ?? [];
  const filtered =
    statusFilter === "all"
      ? allComments
      : allComments.filter((c) => c.status === statusFilter);
  const totalPages = Math.ceil((commentsQuery.data?.total ?? 0) / pageSize);

  return (
    <div className="space-y-6">
      <div className="flex items-center justify-between">
        <h1 className="text-2xl font-bold">{t("comments.title")}</h1>
        <div className="flex items-center gap-2">
          <span className="text-sm text-muted-foreground">
            {commentsQuery.data ? t("comments.total", { count: commentsQuery.data.total }) : ""}
          </span>
        </div>
      </div>

      <div className="flex items-center gap-2 flex-wrap">
        <Filter className="size-4 text-muted-foreground" />
        {["all", ...STATUS_OPTIONS.map((s) => s.value)].map((val) => (
          <Button
            key={val}
            variant={statusFilter === val ? "default" : "outline"}
            size="sm"
            onClick={() => setStatusFilter(val)}
          >
            {val === "all" ? t("comments.all") : val === "pending" ? t("comments.pending") : val === "approved" ? t("comments.approved") : t("comments.rejected")}
          </Button>
        ))}
      </div>

      {selected.size > 0 && (
        <div className="flex items-center gap-2 p-2 bg-muted rounded-md">
          <CheckSquare className="size-4" />
          <span className="text-sm font-medium">{t("comments.selected", { count: selected.size })}</span>
          <Button
            variant="outline"
            size="sm"
            onClick={() => handleBulkStatus("approved")}
            disabled={bulkStatusMutation.isPending}
          >
            {t("comments.approve")}
          </Button>
          <Button
            variant="outline"
            size="sm"
            onClick={() => handleBulkStatus("rejected")}
            disabled={bulkStatusMutation.isPending}
          >
            {t("comments.reject")}
          </Button>
          <Button
            variant="destructive"
            size="sm"
            onClick={handleBulkDelete}
            disabled={bulkDeleteMutation.isPending}
          >
            {t("common.delete")}
          </Button>
        </div>
      )}

      <Card>
        <CardContent className="p-0">
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead className="w-10">
                  <Checkbox
                    checked={
                      filtered.length > 0 &&
                      filtered.every((c) => selected.has(c.id))
                    }
                    onCheckedChange={() =>
                      toggleSelectAll(filtered.map((c) => c.id))
                    }
                  />
                </TableHead>
                <TableHead>{t("comments.author")}</TableHead>
                <TableHead>{t("comments.contentCol")}</TableHead>
                <TableHead>{t("comments.post")}</TableHead>
                <TableHead>{t("common.status")}</TableHead>
                <TableHead>{t("comments.date")}</TableHead>
                <TableHead className="text-right">{t("common.actions")}</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {commentsQuery.isLoading ? (
                <TableRow>
                  <TableCell colSpan={7} className="text-center py-8">
                    {t("common.loading")}
                  </TableCell>
                </TableRow>
              ) : filtered.length === 0 ? (
                <TableRow>
                  <TableCell colSpan={7} className="text-center py-8">
                    {t("comments.noComments")}
                  </TableCell>
                </TableRow>
              ) : (
                filtered.map((c) => (
                  <TableRow key={c.id} className={selected.has(c.id) ? "bg-muted/50" : ""}>
                    <TableCell>
                      <Checkbox
                        checked={selected.has(c.id)}
                        onCheckedChange={() => toggleSelect(c.id)}
                      />
                    </TableCell>
                    <TableCell className="font-medium whitespace-nowrap">
                      {c.nickname || "User"}
                    </TableCell>
                    <TableCell className="max-w-[300px] truncate">
                      {c.content}
                    </TableCell>
                    <TableCell className="whitespace-nowrap">
                      <span className="text-sm">{c.post_title}</span>
                    </TableCell>
                    <TableCell>
                      <Badge variant={statusBadgeVariant(c.status)}>
                        {c.status}
                      </Badge>
                    </TableCell>
                    <TableCell className="whitespace-nowrap">
                      {new Date(c.created_at).toLocaleDateString()}
                    </TableCell>
                    <TableCell className="text-right">
                      <DropdownMenu>
                        <DropdownMenuTrigger className="inline-flex items-center justify-center rounded-md p-1 hover:bg-muted transition-colors">
                          <MoreVertical className="size-4" />
                        </DropdownMenuTrigger>
                        <DropdownMenuContent align="end">
                          {c.status !== "approved" && (
                            <DropdownMenuItem
                              onClick={() =>
                                statusMutation.mutate({ id: c.id, status: "approved" })
                              }
                            >
                              <CheckCircle className="size-4 text-green-600" />
                              {t("comments.approve")}
                            </DropdownMenuItem>
                          )}
                          {c.status !== "rejected" && (
                            <DropdownMenuItem
                              onClick={() =>
                                statusMutation.mutate({ id: c.id, status: "rejected" })
                              }
                            >
                              <XCircle className="size-4 text-red-500" />
                              {t("comments.reject")}
                            </DropdownMenuItem>
                          )}
                          <DropdownMenuItem
                            className="text-destructive"
                            onClick={() => handleDelete(c.id)}
                            disabled={deleteMutation.isPending}
                          >
                            <Trash2 className="size-4" />
                            {t("common.delete")}
                          </DropdownMenuItem>
                        </DropdownMenuContent>
                      </DropdownMenu>
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
