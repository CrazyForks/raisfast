
import { useState } from "react";
import Link from "@/lib/link";
import { useRouter } from "@/lib/navigation";
import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import { Plus, Pencil, Trash2, MoreVertical } from "lucide-react";
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
import { client } from "@/lib/raisfast";
import { SDKError } from "@raisfast/sdk";
import { DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuTrigger } from "@/components/ui/dropdown-menu";
import { useT } from "@/lib/i18n";

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
  const { t } = useT();
  const [page, setPage] = useState(1);
  const [statusFilter, setStatusFilter] = useState<string>("");
  const pageSize = 20;

  const postsQuery = useQuery({
    queryKey: ["admin-posts", page, statusFilter],
    queryFn: () =>
      client.posts.adminList({
        page,
        page_size: pageSize,
        status: statusFilter || undefined,
      }),
  });

  const deleteMutation = useMutation({
    mutationFn: (slug: string) => client.posts.delete(slug),
    onSuccess: () => {
      toast.success(t("posts.postDeleted"));
      queryClient.invalidateQueries({ queryKey: ["admin-posts"] });
    },
    onError: (err) => {
      if (err instanceof SDKError) {
        toast.error(err.message);
      } else {
        toast.error(t("posts.failedToDelete"));
      }
    },
  });

  function handleDelete(slug: string) {
    if (confirm(t("posts.confirmDelete"))) {
      deleteMutation.mutate(slug);
    }
  }

  const posts = postsQuery.data?.items ?? [];
  const totalPages = Math.ceil((postsQuery.data?.total ?? 0) / pageSize);

  return (
    <div className="space-y-6">
      <div className="flex items-center justify-between">
        <h1 className="text-2xl font-bold">{t("posts.title")}</h1>
        <div className="flex items-center gap-2">
          <select
            className="h-9 rounded-md border border-input bg-background px-3 text-sm"
            value={statusFilter}
            onChange={(e) => { setStatusFilter(e.target.value); setPage(1); }}
          >
            <option value="">{t("posts.allStatus")}</option>
            <option value="published">{t("common.published")}</option>
            <option value="draft">{t("common.draft")}</option>
          </select>
          <Link href="/posts/new">
            <Button>
              <Plus className="size-4" />
              {t("posts.newPost")}
            </Button>
          </Link>
        </div>
      </div>

      <Card>
        <CardContent className="p-0">
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead>{t("posts.titleCol")}</TableHead>
                <TableHead>{t("common.status")}</TableHead>
                <TableHead>{t("posts.categoryCol")}</TableHead>
                <TableHead>{t("posts.authorCol")}</TableHead>
                <TableHead>{t("posts.createdCol")}</TableHead>
                <TableHead className="text-right">{t("common.actions")}</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {postsQuery.isLoading ? (
                <TableRow>
                  <TableCell colSpan={6} className="text-center py-8">
                    {t("common.loading")}
                  </TableCell>
                </TableRow>
              ) : posts.length === 0 ? (
                <TableRow>
                  <TableCell colSpan={6} className="text-center py-8">
                    {t("posts.noPosts")}
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
                      <DropdownMenu>
                        <DropdownMenuTrigger
                          className="inline-flex items-center justify-center rounded-md p-1 hover:bg-muted transition-colors"
                        >
                          <MoreVertical className="size-4" />
                        </DropdownMenuTrigger>
                        <DropdownMenuContent align="end">
                          <DropdownMenuItem onClick={() => router.push(`/admin/posts/${post.slug}/edit`)}>
                            <Pencil className="size-4 mr-2" />
                            {t("common.edit")}
                          </DropdownMenuItem>
                          <DropdownMenuItem
                            className="text-destructive"
                            onClick={() => handleDelete(post.slug)}
                            disabled={deleteMutation.isPending}
                          >
                            <Trash2 className="size-4 mr-2" />
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
