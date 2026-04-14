"use client";

import { useState } from "react";
import Link from "next/link";
import { useRouter } from "next/navigation";
import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import { Plus, Pencil, Trash2 } from "lucide-react";
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
import { api, ApiError } from "@/lib/api";

interface Post {
  id: string;
  title: string;
  slug: string;
  status: string;
  category_name: string | null;
  author_name: string | null;
  created_at: string;
}

interface PaginatedData<T> {
  items: T[];
  total: number;
  page: number;
  page_size: number;
}

export default function PostsPage() {
  const router = useRouter();
  const queryClient = useQueryClient();
  const [page, setPage] = useState(1);
  const [statusFilter, setStatusFilter] = useState<string>("");
  const pageSize = 20;

  const postsQuery = useQuery({
    queryKey: ["admin-posts", page, statusFilter],
    queryFn: () =>
      api.get<PaginatedData<Post>>(
        `/admin/posts?page=${page}&page_size=${pageSize}${statusFilter ? `&status=${statusFilter}` : ""}`,
      ),
  });

  const deleteMutation = useMutation({
    mutationFn: (slug: string) => api.delete(`/posts/${slug}`),
    onSuccess: () => {
      toast.success("Post deleted");
      queryClient.invalidateQueries({ queryKey: ["admin-posts"] });
    },
    onError: (err) => {
      if (err instanceof ApiError) {
        toast.error(err.message);
      } else {
        toast.error("Failed to delete post");
      }
    },
  });

  function handleDelete(slug: string) {
    if (confirm("Are you sure you want to delete this post?")) {
      deleteMutation.mutate(slug);
    }
  }

  const posts = postsQuery.data?.items ?? [];
  const totalPages = Math.ceil((postsQuery.data?.total ?? 0) / pageSize);

  return (
    <div className="space-y-6">
      <div className="flex items-center justify-between">
        <h1 className="text-2xl font-bold">Posts</h1>
        <div className="flex items-center gap-2">
          <select
            className="h-9 rounded-md border border-input bg-background px-3 text-sm"
            value={statusFilter}
            onChange={(e) => { setStatusFilter(e.target.value); setPage(1); }}
          >
            <option value="">All Status</option>
            <option value="published">Published</option>
            <option value="draft">Draft</option>
          </select>
          <Link href="/admin/posts/new">
            <Button>
              <Plus className="size-4" />
              New Post
            </Button>
          </Link>
        </div>
      </div>

      <Card>
        <CardContent className="p-0">
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead>Title</TableHead>
                <TableHead>Status</TableHead>
                <TableHead>Category</TableHead>
                <TableHead>Author</TableHead>
                <TableHead>Created</TableHead>
                <TableHead className="text-right">Actions</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {postsQuery.isLoading ? (
                <TableRow>
                  <TableCell colSpan={6} className="text-center py-8">
                    Loading...
                  </TableCell>
                </TableRow>
              ) : posts.length === 0 ? (
                <TableRow>
                  <TableCell colSpan={6} className="text-center py-8">
                    No posts found.
                  </TableCell>
                </TableRow>
              ) : (
                posts.map((post) => (
                  <TableRow key={post.id}>
                    <TableCell className="font-medium max-w-[300px] truncate">
                      {post.title}
                    </TableCell>
                    <TableCell>
                      <Badge
                        variant={
                          post.status === "published"
                            ? "default"
                            : "secondary"
                        }
                      >
                        {post.status}
                      </Badge>
                    </TableCell>
                    <TableCell>{post.category_name || "—"}</TableCell>
                    <TableCell>{post.author_name || "—"}</TableCell>
                    <TableCell>
                      {new Date(post.created_at).toLocaleDateString()}
                    </TableCell>
                    <TableCell className="text-right">
                      <div className="flex items-center justify-end gap-1">
                        <Button
                          variant="ghost"
                          size="icon-sm"
                          onClick={() => router.push(`/admin/posts/${post.slug}/edit`)}
                        >
                          <Pencil className="size-4" />
                        </Button>
                        <Button
                          variant="ghost"
                          size="icon-sm"
                          onClick={() => handleDelete(post.slug)}
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
