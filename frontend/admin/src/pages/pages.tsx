
import { useState } from "react";
import Link from "@/lib/link";
import { useRouter } from "@/lib/navigation";
import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import { Plus, Trash2, MoreVertical, Pencil, Eye, Globe } from "lucide-react";
import { toast } from "sonner";

import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogFooter,
} from "@/components/ui/dialog";
import { DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuTrigger } from "@/components/ui/dropdown-menu";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { Skeleton } from "@/components/ui/skeleton";
import { page as pageApi } from "@/lib/page";
import { PageStatus } from "@raisfast/sdk";
import { useT } from "@/lib/i18n";

const STATUS_COLORS: Record<PageStatus | string, string> = {
  [PageStatus.draft]: "bg-yellow-500",
  [PageStatus.published]: "bg-green-500",
};

export default function PagesListPage() {
  const { t } = useT();
  const router = useRouter();
  const queryClient = useQueryClient();
  const [statusFilter, setStatusFilter] = useState("");
  const [deleteId, setDeleteId] = useState<string | null>(null);

  const listQuery = useQuery({
    queryKey: ["admin-pages", statusFilter],
    queryFn: () => pageApi.adminList(1, 100, statusFilter || undefined),
  });

  const deleteMutation = useMutation({
    mutationFn: (id: string) => pageApi.delete(id),
    onSuccess: () => {
      toast.success(t("pages.pageDeleted"));
      setDeleteId(null);
      queryClient.invalidateQueries({ queryKey: ["admin-pages"] });
    },
    onError: () => toast.error(t("pages.failedToDelete")),
  });

  const statusMutation = useMutation({
    mutationFn: ({ id, status }: { id: string; status: string }) =>
      pageApi.updateStatus(id, status),
    onSuccess: () => {
      toast.success(t("pages.statusUpdated"));
      queryClient.invalidateQueries({ queryKey: ["admin-pages"] });
    },
    onError: () => toast.error(t("pages.failedToUpdateStatus")),
  });

  const items = listQuery.data?.items ?? [];

  return (
    <div className="space-y-6">
      <div className="flex items-center justify-between">
        <h1 className="text-2xl font-bold">{t("pages.title")}</h1>
        <Link href="/pages/new">
          <Button>
            <Plus className="size-4" />
            {t("pages.newPage")}
          </Button>
        </Link>
      </div>

      <div className="flex gap-1 bg-muted rounded-lg p-[3px] w-fit">
        {["", PageStatus.draft, PageStatus.published].map((s) => (
          <button
            key={s}
            type="button"
            onClick={() => setStatusFilter(s)}
            className={`px-3 py-1 text-sm rounded-md transition-colors ${
              statusFilter === s
                ? "bg-background text-foreground shadow-sm"
                : "text-muted-foreground hover:text-foreground"
            }`}
          >
            {s ? s.charAt(0).toUpperCase() + s.slice(1) : t("common.all")}
          </button>
        ))}
      </div>

      <Card>
        <CardContent className="p-0">
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead>{t("pages.title")}</TableHead>
                <TableHead>{t("pages.slug")}</TableHead>
                <TableHead>{t("common.status")}</TableHead>
                <TableHead>{t("pages.template")}</TableHead>
                <TableHead>{t("pages.createdAt")}</TableHead>
                <TableHead className="text-right">{t("common.actions")}</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {listQuery.isLoading ? (
                <TableRow>
                  <TableCell colSpan={6} className="text-center py-8">{t("common.loading")}</TableCell>
                </TableRow>
              ) : !items.length ? (
                <TableRow>
                  <TableCell colSpan={6} className="text-center py-8">{t("pages.noPages")}</TableCell>
                </TableRow>
              ) : (
                items.map((p) => (
                  <TableRow key={p.id}>
                    <TableCell className="font-medium">{p.title}</TableCell>
                    <TableCell className="text-muted-foreground">/{p.slug}</TableCell>
                    <TableCell>
                      <div className="flex items-center gap-1.5">
                        <div className={`size-2 rounded-full ${STATUS_COLORS[p.status] ?? "bg-gray-300"}`} />
                        <span className="text-sm capitalize">{p.status}</span>
                      </div>
                    </TableCell>
                    <TableCell><Badge variant="secondary">{p.template}</Badge></TableCell>
                    <TableCell className="text-muted-foreground text-sm">{new Date(p.created_at).toLocaleDateString()}</TableCell>
                    <TableCell className="text-right">
                      <DropdownMenu>
                        <DropdownMenuTrigger className="inline-flex items-center justify-center rounded-md p-1 hover:bg-muted transition-colors">
                          <MoreVertical className="size-4" />
                        </DropdownMenuTrigger>
                        <DropdownMenuContent align="end">
                          <DropdownMenuItem onClick={() => router.push(`/admin/pages/${p.id}/edit`)}>
                            <Pencil className="size-4 mr-2" />{t("common.edit")}
                          </DropdownMenuItem>
                          <DropdownMenuItem onClick={() => window.open(`/pages/${p.slug}`, "_blank")}>
                            <Eye className="size-4 mr-2" />{t("pages.preview")}
                          </DropdownMenuItem>
                          {p.status === PageStatus.draft && (
                            <DropdownMenuItem onClick={() => statusMutation.mutate({ id: p.id, status: PageStatus.published })}>
                              <Eye className="size-4 mr-2" />
                              {t("common.publish")}
                            </DropdownMenuItem>
                          )}
                          {p.status === PageStatus.published && (
                            <DropdownMenuItem onClick={() => statusMutation.mutate({ id: p.id, status: PageStatus.draft })}>
                              {t("pages.unpublish")}
                            </DropdownMenuItem>
                          )}
                          <DropdownMenuItem className="text-destructive" onClick={() => setDeleteId(p.id)} disabled={deleteMutation.isPending}>
                            <Trash2 className="size-4 mr-2" />{t("common.delete")}
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

      <Dialog open={!!deleteId} onOpenChange={() => setDeleteId(null)}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>{t("pages.confirmDelete")}</DialogTitle>
          </DialogHeader>
          <DialogFooter>
            <Button variant="outline" onClick={() => setDeleteId(null)}>{t("common.cancel")}</Button>
            <Button variant="destructive" onClick={() => deleteId && deleteMutation.mutate(deleteId)} disabled={deleteMutation.isPending}>
              {t("common.delete")}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </div>
  );
}
