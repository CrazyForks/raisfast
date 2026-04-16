"use client";

import { useState } from "react";
import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import {
  CheckCircle,
  XCircle,
  Trash2,
  MessageSquare,
  Filter,
  CheckSquare,
} from "lucide-react";
import { toast } from "sonner";
import Link from "next/link";

import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { Card, CardContent } from "@/components/ui/card";
import { Checkbox } from "@/components/ui/checkbox";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { api, ApiError } from "@/lib/api";
import { useAuthStore } from "@/stores/auth";

interface AdminComment {
  id: string;
  post_id: string;
  post_title: string;
  author_id: string | null;
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
  const { isAdmin } = useAuthStore();
  const queryClient = useQueryClient();
  const [page, setPage] = useState(1);
  const [statusFilter, setStatusFilter] = useState<string>("all");
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const pageSize = 20;

  const commentsQuery = useQuery({
    queryKey: ["admin-comments", page],
    queryFn: () =>
      api.get<PaginatedData<AdminComment>>(
        `/comments?page=${page}&page_size=${pageSize}`,
      ),
    enabled: isAdmin(),
  });

  const statusMutation = useMutation({
    mutationFn: ({ id, status }: { id: string; status: string }) =>
      api.put(`/comments/${id}/status`, { status }),
    onSuccess: () => {
      toast.success("Comment status updated");
      queryClient.invalidateQueries({ queryKey: ["admin-comments"] });
    },
    onError: (err) => {
      if (err instanceof ApiError) {
        toast.error(err.message);
      } else {
        toast.error("Failed to update status");
      }
    },
  });

  const deleteMutation = useMutation({
    mutationFn: (id: string) => api.delete(`/comments/${id}`),
    onSuccess: () => {
      toast.success("Comment deleted");
      queryClient.invalidateQueries({ queryKey: ["admin-comments"] });
      setSelected(new Set());
    },
    onError: (err) => {
      if (err instanceof ApiError) {
        toast.error(err.message);
      } else {
        toast.error("Failed to delete comment");
      }
    },
  });

  const bulkStatusMutation = useMutation({
    mutationFn: ({ ids, status }: { ids: string[]; status: string }) =>
      Promise.all(
        ids.map((id) => api.put(`/comments/${id}/status`, { status })),
      ),
    onSuccess: (_data, vars) => {
      toast.success(`${vars.ids.length} comment(s) ${vars.status}`);
      queryClient.invalidateQueries({ queryKey: ["admin-comments"] });
      setSelected(new Set());
    },
    onError: (err) => {
      if (err instanceof ApiError) {
        toast.error(err.message);
      } else {
        toast.error("Bulk action failed");
      }
    },
  });

  const bulkDeleteMutation = useMutation({
    mutationFn: (ids: string[]) =>
      Promise.all(ids.map((id) => api.delete(`/comments/${id}`))),
    onSuccess: (_data, ids) => {
      toast.success(`${ids.length} comment(s) deleted`);
      queryClient.invalidateQueries({ queryKey: ["admin-comments"] });
      setSelected(new Set());
    },
    onError: (err) => {
      if (err instanceof ApiError) {
        toast.error(err.message);
      } else {
        toast.error("Bulk delete failed");
      }
    },
  });

  function handleDelete(id: string) {
    if (confirm("Are you sure you want to delete this comment?")) {
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
    if (confirm(`Delete ${selected.size} selected comment(s)?`)) {
      bulkDeleteMutation.mutate(Array.from(selected));
    }
  }

  if (!isAdmin()) {
    return (
      <div className="flex min-h-[50vh] items-center justify-center">
        <div className="text-center space-y-4">
          <MessageSquare className="size-12 mx-auto text-muted-foreground" />
          <h2 className="text-xl font-semibold">Admin Only</h2>
          <p className="text-muted-foreground">
            Only administrators can manage comments.
          </p>
          <Link href="/admin/dashboard">
            <Button variant="outline">Back to Dashboard</Button>
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
        <h1 className="text-2xl font-bold">Comments</h1>
        <div className="flex items-center gap-2">
          <span className="text-sm text-muted-foreground">
            {commentsQuery.data ? `${commentsQuery.data.total} total` : ""}
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
            {val === "all" ? "All" : STATUS_OPTIONS.find((s) => s.value === val)?.label}
          </Button>
        ))}
      </div>

      {selected.size > 0 && (
        <div className="flex items-center gap-2 p-2 bg-muted rounded-md">
          <CheckSquare className="size-4" />
          <span className="text-sm font-medium">{selected.size} selected</span>
          <Button
            variant="outline"
            size="sm"
            onClick={() => handleBulkStatus("approved")}
            disabled={bulkStatusMutation.isPending}
          >
            Approve
          </Button>
          <Button
            variant="outline"
            size="sm"
            onClick={() => handleBulkStatus("rejected")}
            disabled={bulkStatusMutation.isPending}
          >
            Reject
          </Button>
          <Button
            variant="destructive"
            size="sm"
            onClick={handleBulkDelete}
            disabled={bulkDeleteMutation.isPending}
          >
            Delete
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
                <TableHead>Author</TableHead>
                <TableHead>Content</TableHead>
                <TableHead>Post</TableHead>
                <TableHead>Status</TableHead>
                <TableHead>Date</TableHead>
                <TableHead className="text-right">Actions</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {commentsQuery.isLoading ? (
                <TableRow>
                  <TableCell colSpan={7} className="text-center py-8">
                    Loading...
                  </TableCell>
                </TableRow>
              ) : filtered.length === 0 ? (
                <TableRow>
                  <TableCell colSpan={7} className="text-center py-8">
                    No comments found.
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
                      <div className="flex items-center justify-end gap-1">
                        {c.status !== "approved" && (
                          <Button
                            variant="ghost"
                            size="icon-sm"
                            title="Approve"
                            onClick={() =>
                              statusMutation.mutate({ id: c.id, status: "approved" })
                            }
                          >
                            <CheckCircle className="size-4 text-green-600" />
                          </Button>
                        )}
                        {c.status !== "rejected" && (
                          <Button
                            variant="ghost"
                            size="icon-sm"
                            title="Reject"
                            onClick={() =>
                              statusMutation.mutate({ id: c.id, status: "rejected" })
                            }
                          >
                            <XCircle className="size-4 text-red-500" />
                          </Button>
                        )}
                        <Button
                          variant="ghost"
                          size="icon-sm"
                          title="Delete"
                          onClick={() => handleDelete(c.id)}
                          disabled={deleteMutation.isPending}
                        >
                          <Trash2 className="size-4" />
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
