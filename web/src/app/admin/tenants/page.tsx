"use client";

import { useState } from "react";
import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import { Plus, Trash2, Pencil, Save, X, ShieldCheck, ShieldAlert } from "lucide-react";
import { useT } from "@/lib/i18n";
import { useForm } from "react-hook-form";
import { z } from "zod";
import { zodResolver } from "@hookform/resolvers/zod";
import { toast } from "sonner";

import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
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
import { Badge } from "@/components/ui/badge";
import { api, ApiError } from "@/lib/api";

interface Tenant {
  id: string;
  name: string;
  domain: string | null;
  config: string;
  status: string;
  created_at: string;
  updated_at: string;
}

interface PaginatedData<T> {
  items: T[];
  total: number;
  page: number;
  page_size: number;
}

const tenantSchema = z.object({
  name: z.string().min(1, "Name is required"),
  domain: z.string().optional(),
});

type TenantForm = z.infer<typeof tenantSchema>;

export default function TenantsPage() {
  const { t } = useT();
  const queryClient = useQueryClient();
  const [dialogOpen, setDialogOpen] = useState(false);
  const [editTenant, setEditTenant] = useState<Tenant | null>(null);
  const [editName, setEditName] = useState("");
  const [editDomain, setEditDomain] = useState("");
  const [editStatus, setEditStatus] = useState("");
  const [page, setPage] = useState(1);
  const pageSize = 20;

  const tenantsQuery = useQuery({
    queryKey: ["tenants", page],
    queryFn: () =>
      api.get<PaginatedData<Tenant>>(`/admin/tenants?page=${page}&page_size=${pageSize}`),
  });

  const {
    register,
    handleSubmit,
    reset,
    formState: { errors },
  } = useForm<TenantForm>({
    resolver: zodResolver(tenantSchema as never),
    defaultValues: { name: "", domain: "" },
  });

  const createMutation = useMutation({
    mutationFn: (data: TenantForm) =>
      api.post("/admin/tenants", {
        name: data.name,
        domain: data.domain || null,
      }),
    onSuccess: () => {
      toast.success(t("tenants.tenantCreated"));
      queryClient.invalidateQueries({ queryKey: ["tenants"] });
      setDialogOpen(false);
      reset();
    },
    onError: (err) => {
      if (err instanceof ApiError) {
        toast.error(err.message);
      } else {
        toast.error(t("tenants.failedToCreate"));
      }
    },
  });

  const updateMutation = useMutation({
    mutationFn: ({
      id,
      data,
    }: {
      id: string;
      data: { name?: string; domain?: string; status?: string };
    }) => api.put(`/admin/tenants/${id}`, data),
    onSuccess: () => {
      toast.success(t("tenants.tenantUpdated"));
      queryClient.invalidateQueries({ queryKey: ["tenants"] });
      setEditTenant(null);
    },
    onError: (err) => {
      if (err instanceof ApiError) {
        toast.error(err.message);
      } else {
        toast.error(t("tenants.failedToUpdate"));
      }
    },
  });

  const deleteMutation = useMutation({
    mutationFn: (id: string) => api.delete(`/admin/tenants/${id}`),
    onSuccess: () => {
      toast.success(t("tenants.tenantDeleted"));
      queryClient.invalidateQueries({ queryKey: ["tenants"] });
    },
    onError: (err) => {
      if (err instanceof ApiError) {
        toast.error(err.message);
      } else {
        toast.error(t("tenants.failedToDelete"));
      }
    },
  });

  function handleDelete(id: string) {
    if (id === "default") {
      toast.error(t("tenants.cannotDeleteDefault"));
      return;
    }
    if (confirm(t("tenants.confirmDeleteMsg"))) {
      deleteMutation.mutate(id);
    }
  }

  function startEdit(t: Tenant) {
    setEditTenant(t);
    setEditName(t.name);
    setEditDomain(t.domain ?? "");
    setEditStatus(t.status);
  }

  function saveEdit() {
    if (!editTenant) return;
    updateMutation.mutate({
      id: editTenant.id,
      data: {
        name: editName,
        domain: editDomain || undefined,
        status: editStatus,
      },
    });
  }

  const tenants = tenantsQuery.data?.items ?? [];
  const totalPages = Math.ceil((tenantsQuery.data?.total ?? 0) / pageSize);

  return (
    <div className="space-y-6">
      <div className="flex items-center justify-between">
        <h1 className="text-2xl font-bold">{t("tenants.title")}</h1>
        <Dialog open={dialogOpen} onOpenChange={setDialogOpen}>
          <DialogTrigger render={<Button />}>
            <Plus className="size-4" />
            {t("tenants.newTenant")}
          </DialogTrigger>
          <DialogContent>
            <DialogHeader>
              <DialogTitle>{t("tenants.newTenant")}</DialogTitle>
              <DialogDescription>
                {t("tenants.createTenant")}
              </DialogDescription>
            </DialogHeader>
            <form
              onSubmit={handleSubmit((data) => createMutation.mutate(data))}
              className="space-y-4"
            >
              <div className="space-y-2">
                <Label htmlFor="tenant-name">{t("common.name")}</Label>
                <Input
                  id="tenant-name"
                  placeholder="Acme Corp"
                  {...register("name")}
                />
                {errors.name && (
                  <p className="text-sm text-red-500">{errors.name.message}</p>
                )}
              </div>
              <div className="space-y-2">
                <Label htmlFor="tenant-domain">{t("tenants.domain")}</Label>
                <Input
                  id="tenant-domain"
                  placeholder="acme.example.com"
                  {...register("domain")}
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
                <TableHead>ID</TableHead>
                <TableHead>{t("common.name")}</TableHead>
                <TableHead>{t("tenants.domain")}</TableHead>
                <TableHead>{t("common.status")}</TableHead>
                <TableHead>{t("tenants.created")}</TableHead>
                <TableHead className="text-right">{t("common.actions")}</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {tenantsQuery.isLoading ? (
                <TableRow>
                  <TableCell colSpan={6} className="text-center py-8">
                    {t("common.loading")}
                  </TableCell>
                </TableRow>
              ) : tenants.length === 0 ? (
                <TableRow>
                  <TableCell colSpan={6} className="text-center py-8">
                    {t("tenants.noTenants")}
                  </TableCell>
                </TableRow>
              ) : (
                tenants.map((t) => (
                  <TableRow key={t.id}>
                    <TableCell className="font-mono text-xs">
                      {t.id === "default" ? (
                        <Badge variant="secondary">default</Badge>
                      ) : (
                        t.id.slice(0, 8) + "..."
                      )}
                    </TableCell>
                    <TableCell>
                      {editTenant?.id === t.id ? (
                        <Input
                          value={editName}
                          onChange={(e) => setEditName(e.target.value)}
                          className="h-8 w-40"
                        />
                      ) : (
                        <span className="font-medium">{t.name}</span>
                      )}
                    </TableCell>
                    <TableCell>
                      {editTenant?.id === t.id ? (
                        <Input
                          value={editDomain}
                          onChange={(e) => setEditDomain(e.target.value)}
                          className="h-8 w-48"
                          placeholder="—"
                        />
                      ) : (
                        <span className="text-sm text-muted-foreground">
                          {t.domain || "—"}
                        </span>
                      )}
                    </TableCell>
                    <TableCell>
                      {editTenant?.id === t.id ? (
                        <select
                          value={editStatus}
                          onChange={(e) => setEditStatus(e.target.value)}
                          className="h-8 rounded-md border border-input bg-background px-2 text-sm"
                        >
                          <option value="active">active</option>
                          <option value="suspended">suspended</option>
                        </select>
                      ) : t.status === "active" ? (
                        <Badge variant="default" className="gap-1">
                          <ShieldCheck className="size-3" />
                          active
                        </Badge>
                      ) : (
                        <Badge variant="destructive" className="gap-1">
                          <ShieldAlert className="size-3" />
                          {t.status}
                        </Badge>
                      )}
                    </TableCell>
                    <TableCell>
                      {new Date(t.created_at).toLocaleDateString()}
                    </TableCell>
                    <TableCell className="text-right">
                      {editTenant?.id === t.id ? (
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
                            onClick={() => setEditTenant(null)}
                          >
                            <X className="size-4" />
                          </Button>
                        </div>
                      ) : (
                        <div className="flex items-center justify-end gap-1">
                          <Button
                            variant="ghost"
                            size="icon-sm"
                            onClick={() => startEdit(t)}
                          >
                            <Pencil className="size-4" />
                          </Button>
                          <Button
                            variant="ghost"
                            size="icon-sm"
                            onClick={() => handleDelete(t.id)}
                            disabled={deleteMutation.isPending || t.id === "default"}
                          >
                            <Trash2 className="size-4" />
                          </Button>
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
