"use client";

import { useState } from "react";
import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import { CheckCircle, XCircle, Trash2, MessageSquare } from "lucide-react";
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

export default function CommentsPage() {
  const { isAdmin } = useAuthStore();
  const queryClient = useQueryClient();
  const [page, setPage] = useState(1);
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
    },
    onError: (err) => {
      if (err instanceof ApiError) {
        toast.error(err.message);
      } else {
        toast.error("Failed to delete comment");
      }
    },
  });

  function handleDelete(id: string) {
    if (confirm("Are you sure you want to delete this comment?")) {
      deleteMutation.mutate(id);
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

  const comments = commentsQuery.data?.items ?? [];
  const totalPages = Math.ceil((commentsQuery.data?.total ?? 0) / pageSize);

  return (
    <div className="space-y-6">
      <div className="flex items-center justify-between">
        <h1 className="text-2xl font-bold">Comments</h1>
        <p className="text-sm text-muted-foreground">
          {commentsQuery.data ? `${commentsQuery.data.total} total` : ""}
        </p>
      </div>

      <Card>
        <CardContent className="p-0">
          <Table>
            <TableHeader>
              <TableRow>
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
                  <TableCell colSpan={6} className="text-center py-8">
                    Loading...
                  </TableCell>
                </TableRow>
              ) : comments.length === 0 ? (
                <TableRow>
                  <TableCell colSpan={6} className="text-center py-8">
                    No comments found.
                  </TableCell>
                </TableRow>
              ) : (
                comments.map((c) => (
                  <TableRow key={c.id}>
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
