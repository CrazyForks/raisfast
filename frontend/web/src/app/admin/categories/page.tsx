"use client";

import { useState } from "react";
import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import { Plus, Trash2, Pencil, Save, X, MoreVertical } from "lucide-react";
import { useForm } from "react-hook-form";
import { z } from "zod";
import { zodResolver } from "@hookform/resolvers/zod";
import { toast } from "sonner";

import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Textarea } from "@/components/ui/textarea";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
  DialogTrigger,
} from "@/components/ui/dialog";
import { DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuTrigger } from "@/components/ui/dropdown-menu";
import { client } from "@/lib/raisfast";
import { SDKError } from "@raisfast/sdk";
import { useT } from "@/lib/i18n";

interface Category {
  id: string;
  name: string;
  slug: string;
  description: string | null;
  sort_order: number;
  created_at: string;
}

interface PaginatedData<T> {
  items: T[];
  total: number;
  page: number;
  page_size: number;
}

const categorySchema = z.object({
  name: z.string().min(1, "Name is required"),
  description: z.string().optional(),
  sort_order: z.number().int().min(0),
});

type CategoryForm = z.infer<typeof categorySchema>;

export default function CategoriesPage() {
  const { t } = useT();
  const queryClient = useQueryClient();
  const [dialogOpen, setDialogOpen] = useState(false);
  const [editCat, setEditCat] = useState<Category | null>(null);
  const [editName, setEditName] = useState("");
  const [editDesc, setEditDesc] = useState("");
  const [editSort, setEditSort] = useState(0);
  const [page, setPage] = useState(1);
  const pageSize = 20;

  const categoriesQuery = useQuery({
    queryKey: ["categories", page],
    queryFn: () =>
      client.send<PaginatedData<Category>>(`/categories?page=${page}&page_size=${pageSize}`),
  });

  type FormValues = z.infer<typeof categorySchema>;

  const {
    register,
    handleSubmit,
    reset,
    formState: { errors },
  } = useForm<FormValues>({
    resolver: zodResolver(categorySchema as never),
    defaultValues: { name: "", description: "", sort_order: 0 },
  });

  const createMutation = useMutation({
    mutationFn: (data: CategoryForm) =>
      client.categories.create({
        name: data.name,
        description: data.description || undefined,
        sort_order: data.sort_order,
      }),
    onSuccess: () => {
      toast.success(t("categories.categoryCreated"));
      queryClient.invalidateQueries({ queryKey: ["categories"] });
      setDialogOpen(false);
      reset();
    },
    onError: (err) => {
      if (err instanceof SDKError) {
        toast.error(err.message);
      } else {
        toast.error(t("categories.failedToCreate"));
      }
    },
  });

  const updateMutation = useMutation({
    mutationFn: ({
      id,
      data,
    }: {
      id: string;
      data: { name?: string; description?: string; sort_order?: number };
    }) => client.categories.update(id, data),
    onSuccess: () => {
      toast.success(t("categories.categoryUpdated"));
      queryClient.invalidateQueries({ queryKey: ["categories"] });
      setEditCat(null);
    },
    onError: (err) => {
      if (err instanceof SDKError) {
        toast.error(err.message);
      } else {
        toast.error(t("categories.failedToUpdate"));
      }
    },
  });

  const deleteMutation = useMutation({
    mutationFn: (id: string) => client.categories.delete(id),
    onSuccess: () => {
      toast.success(t("categories.categoryDeleted"));
      queryClient.invalidateQueries({ queryKey: ["categories"] });
    },
    onError: (err) => {
      if (err instanceof SDKError) {
        toast.error(err.message);
      } else {
        toast.error(t("categories.failedToDelete"));
      }
    },
  });

  function handleDelete(id: string) {
    if (confirm(t("categories.confirmDelete"))) {
      deleteMutation.mutate(id);
    }
  }

  function startEdit(cat: Category) {
    setEditCat(cat);
    setEditName(cat.name);
    setEditDesc(cat.description ?? "");
    setEditSort(cat.sort_order);
  }

  function saveEdit() {
    if (!editCat) return;
    updateMutation.mutate({
      id: editCat.id,
      data: {
        name: editName,
        description: editDesc || undefined,
        sort_order: editSort,
      },
    });
  }

  const categories = categoriesQuery.data?.items ?? [];
  const totalPages = Math.ceil((categoriesQuery.data?.total ?? 0) / pageSize);

  return (
    <div className="space-y-6">
      <div className="flex items-center justify-between">
        <h1 className="text-2xl font-bold">{t("categories.title")}</h1>
        <Dialog open={dialogOpen} onOpenChange={setDialogOpen}>
          <DialogTrigger render={<Button />}>
            <Plus className="size-4" />
            {t("categories.newCategory")}
          </DialogTrigger>
          <DialogContent>
            <DialogHeader>
              <DialogTitle>{t("categories.newCategory")}</DialogTitle>
              <DialogDescription>
                {t("categories.createCategory")}
              </DialogDescription>
            </DialogHeader>
            <form
              onSubmit={handleSubmit((data) => createMutation.mutate(data))}
              className="space-y-4"
            >
              <div className="space-y-2">
                <Label htmlFor="cat-name">{t("common.name")}</Label>
                <Input
                  id="cat-name"
                  placeholder={t("categories.categoryName")}
                  {...register("name")}
                />
                {errors.name && (
                  <p className="text-sm text-red-500">{errors.name.message}</p>
                )}
              </div>
              <div className="space-y-2">
                <Label htmlFor="cat-desc">{t("common.description")}</Label>
                <Textarea
                  id="cat-desc"
                  placeholder={t("categories.optionalDesc")}
                  rows={3}
                  {...register("description")}
                />
              </div>
              <div className="space-y-2">
                <Label htmlFor="cat-sort">{t("categories.sortOrder")}</Label>
                <Input
                  id="cat-sort"
                  type="number"
                  {...register("sort_order")}
                />
              </div>
              <DialogFooter>
                <Button
                  type="button"
                  variant="outline"
                  onClick={() => setDialogOpen(false)}
                >
                  {t("common.cancel")}
                </Button>
                <Button type="submit" disabled={createMutation.isPending}>
                  {createMutation.isPending ? t("common.creating") : t("common.create")}
                </Button>
              </DialogFooter>
            </form>
          </DialogContent>
        </Dialog>
      </div>

      <Card>
        <CardContent className="p-0">
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead>{t("common.name")}</TableHead>
                <TableHead>{t("categories.slug")}</TableHead>
                <TableHead>{t("common.description")}</TableHead>
                <TableHead>{t("categories.sortOrder")}</TableHead>
                <TableHead className="text-right">{t("common.actions")}</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {categoriesQuery.isLoading ? (
                <TableRow>
                  <TableCell colSpan={5} className="text-center py-8">
                    {t("common.loading")}
                  </TableCell>
                </TableRow>
              ) : categories.length === 0 ? (
                <TableRow>
                  <TableCell colSpan={5} className="text-center py-8">
                    {t("categories.noCategories")}
                  </TableCell>
                </TableRow>
              ) : (
                categories.map((cat) => (
                  <TableRow key={cat.id}>
                    <TableCell>
                      {editCat?.id === cat.id ? (
                        <Input
                          value={editName}
                          onChange={(e) => setEditName(e.target.value)}
                          className="h-8 w-40"
                        />
                      ) : (
                        <span className="font-medium">{cat.name}</span>
                      )}
                    </TableCell>
                    <TableCell>{cat.slug}</TableCell>
                    <TableCell>
                      {editCat?.id === cat.id ? (
                        <Input
                          value={editDesc}
                          onChange={(e) => setEditDesc(e.target.value)}
                          className="h-8 w-48"
                          placeholder="—"
                        />
                      ) : (
                        <span className="max-w-[200px] truncate block text-muted-foreground">
                          {cat.description || "—"}
                        </span>
                      )}
                    </TableCell>
                    <TableCell>
                      {editCat?.id === cat.id ? (
                        <Input
                          type="number"
                          value={editSort}
                          onChange={(e) => setEditSort(Number(e.target.value))}
                          className="h-8 w-20"
                        />
                      ) : (
                        cat.sort_order
                      )}
                    </TableCell>
                    <TableCell className="text-right">
                      {editCat?.id === cat.id ? (
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
                            onClick={() => setEditCat(null)}
                          >
                            <X className="size-4" />
                          </Button>
                        </div>
                      ) : (
                        <DropdownMenu>
                          <DropdownMenuTrigger className="inline-flex items-center justify-center rounded-md p-1 hover:bg-muted transition-colors">
                            <MoreVertical className="size-4" />
                          </DropdownMenuTrigger>
                          <DropdownMenuContent align="end">
                            <DropdownMenuItem onClick={() => startEdit(cat)}>
                              <Pencil className="size-4 mr-2" />
                              {t("common.edit")}
                            </DropdownMenuItem>
                            <DropdownMenuItem className="text-destructive" onClick={() => handleDelete(cat.id)} disabled={deleteMutation.isPending}>
                              <Trash2 className="size-4 mr-2" />
                              {t("common.delete")}
                            </DropdownMenuItem>
                          </DropdownMenuContent>
                        </DropdownMenu>
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
