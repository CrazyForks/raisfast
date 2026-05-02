"use client";

import { useState } from "react";
import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import { Shield, Plus, Pencil, Trash2, Key, MoreVertical } from "lucide-react";
import { useT } from "@/lib/i18n";
import { useForm } from "react-hook-form";
import { z } from "zod";
import { zodResolver } from "@hookform/resolvers/zod";
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
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Textarea } from "@/components/ui/textarea";
import { Separator } from "@/components/ui/separator";
import { DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuTrigger } from "@/components/ui/dropdown-menu";
import { client } from "@/lib/raisfast";
import { SDKError } from "@raisfast/sdk";
import { useAuthStore } from "@/stores/auth";

interface Role {
  id: string;
  name: string;
  description: string | null;
  is_system: boolean;
  created_at: string;
  updated_at: string;
}

interface PermissionView {
  id: string;
  role_id: string;
  action: string;
  subject: string;
  fields: string[] | null;
  conditions: Record<string, string> | null;
  created_at: string;
}

interface PaginatedData<T> {
  items: T[];
  total: number;
  page: number;
  page_size: number;
}

const roleSchema = z.object({
  name: z.string().min(1, "Name is required").max(50),
  description: z.string().max(200).optional(),
});

type RoleForm = z.infer<typeof roleSchema>;

const permSchema = z.object({
  action: z.string().min(1, "Action is required"),
  subject: z.string().min(1, "Subject is required"),
  fields: z.string().optional(),
});

type PermForm = z.infer<typeof permSchema>;

const COMMON_ACTIONS = ["*", "create", "read", "update", "delete", "publish"];
const COMMON_SUBJECTS = [
  "*",
  "content-type::post",
  "content-type::page",
  "content-type::comment",
  "content-type::media",
  "content-type::category",
  "content-type::tag",
  "content-type::user",
  "admin::*",
];

export default function RbacPage() {
  const { t } = useT();
  const { isAdmin } = useAuthStore();
  const queryClient = useQueryClient();

  const [selectedRoleId, setSelectedRoleId] = useState<string | null>(null);
  const [createOpen, setCreateOpen] = useState(false);
  const [editRole, setEditRole] = useState<Role | null>(null);
  const [deleteRole, setDeleteRole] = useState<Role | null>(null);
  const [addPermOpen, setAddPermOpen] = useState(false);
  const [removePerm, setRemovePerm] = useState<PermissionView | null>(null);
  const [page, setPage] = useState(1);
  const pageSize = 20;

  const rolesQuery = useQuery({
    queryKey: ["rbac-roles", page],
    queryFn: () =>
      client.send<PaginatedData<Role>>("/admin/rbac/roles", { query: { page: String(page), page_size: String(pageSize) } }),
  });

  const permsQuery = useQuery({
    queryKey: ["rbac-permissions", selectedRoleId],
    queryFn: () =>
      client.send<PermissionView[]>(
        `/admin/rbac/roles/${selectedRoleId}/permissions`,
      ),
    enabled: !!selectedRoleId,
  });

  const {
    register: regRole,
    handleSubmit: submitRole,
    reset: resetRole,
    formState: { errors: roleErrors },
  } = useForm<RoleForm>({
    resolver: zodResolver(roleSchema as never),
  });

  const {
    register: regPerm,
    handleSubmit: submitPerm,
    reset: resetPerm,
    formState: { errors: permErrors },
  } = useForm<PermForm>({
    resolver: zodResolver(permSchema as never),
    defaultValues: { action: "", subject: "", fields: "" },
  });

  const roles = rolesQuery.data?.items ?? [];
  const totalPages = Math.ceil((rolesQuery.data?.total ?? 0) / pageSize);
  const permissions = permsQuery.data ?? [];
  const selectedRole = roles.find((r) => r.id === selectedRoleId);

  const createMutation = useMutation({
    mutationFn: (data: RoleForm) =>
      client.send<Role>("/admin/rbac/roles", { method: "POST", body: data }),
    onSuccess: (role) => {
      toast.success(t("rbac.roleCreated"));
      queryClient.invalidateQueries({ queryKey: ["rbac-roles"] });
      setCreateOpen(false);
      resetRole();
      setSelectedRoleId(role.id);
    },
    onError: (err) => {
        toast.error(
          err instanceof SDKError ? err.message : t("rbac.failedToCreate"),
        );
    },
  });

  const updateMutation = useMutation({
    mutationFn: ({ id, ...data }: RoleForm & { id: string }) =>
      client.send<Role>(`/admin/rbac/roles/${id}`, { method: "PUT", body: data }),
    onSuccess: () => {
      toast.success(t("rbac.roleUpdated"));
      queryClient.invalidateQueries({ queryKey: ["rbac-roles"] });
      setEditRole(null);
      resetRole();
    },
    onError: (err) => {
        toast.error(
          err instanceof SDKError ? err.message : t("rbac.failedToUpdate"),
        );
    },
  });

  const deleteMutation = useMutation({
    mutationFn: (id: string) => client.send(`/admin/rbac/roles/${id}`, { method: "DELETE" }),
    onSuccess: () => {
      toast.success(t("rbac.roleDeleted"));
      queryClient.invalidateQueries({ queryKey: ["rbac-roles"] });
      if (selectedRoleId === deleteRole?.id) setSelectedRoleId(null);
      setDeleteRole(null);
    },
    onError: (err) => {
        toast.error(
          err instanceof SDKError ? err.message : t("rbac.failedToDelete"),
        );
    },
  });

  const addPermMutation = useMutation({
    mutationFn: (
      entries: {
        action: string;
        subject: string;
        fields?: string[];
      }[],
    ) =>
      client.send<PermissionView[]>(
        `/admin/rbac/roles/${selectedRoleId}/permissions`,
        { method: "PUT", body: { permissions: entries } },
      ),
    onSuccess: () => {
      toast.success(t("rbac.permissionAdded"));
      queryClient.invalidateQueries({
        queryKey: ["rbac-permissions", selectedRoleId],
      });
      setAddPermOpen(false);
      resetPerm();
    },
    onError: (err) => {
      toast.error(err instanceof SDKError ? err.message : "Failed");
    },
  });

  const removePermMutation = useMutation({
    mutationFn: () => {
      const remaining = permissions.filter((p) => p.id !== removePerm!.id);
      return client.send<PermissionView[]>(
        `/admin/rbac/roles/${selectedRoleId}/permissions`,
        {
          method: "PUT",
          body: {
            permissions: remaining.map((p) => ({
              action: p.action,
              subject: p.subject,
              fields: p.fields,
              conditions: p.conditions,
            })),
          },
        },
      );
    },
    onSuccess: () => {
      toast.success(t("rbac.permissionRemoved"));
      queryClient.invalidateQueries({
        queryKey: ["rbac-permissions", selectedRoleId],
      });
      setRemovePerm(null);
    },
    onError: (err) => {
      toast.error(err instanceof SDKError ? err.message : "Failed");
    },
  });

  function openEdit(role: Role) {
    setEditRole(role);
    resetRole({ name: role.name, description: role.description ?? "" });
  }

  function openCreate() {
    resetRole({ name: "", description: "" });
    setCreateOpen(true);
  }

  function onSubmitPerm(data: PermForm) {
    const newPerm = {
      action: data.action,
      subject: data.subject,
      ...(data.fields
        ? { fields: data.fields.split(",").map((s) => s.trim()).filter(Boolean) }
        : {}),
    };
    const all = [
      ...permissions.map((p) => ({
        action: p.action,
        subject: p.subject,
        fields: p.fields ?? undefined,
        conditions: p.conditions ?? undefined,
      })),
      newPerm,
    ];
    addPermMutation.mutate(all);
  }

  const existingActions = [...new Set(permissions.map((p) => p.action))];
  const existingSubjects = [...new Set(permissions.map((p) => p.subject))];

  if (!isAdmin()) {
    return (
      <div className="flex min-h-[50vh] items-center justify-center">
        <div className="text-center space-y-4">
          <Shield className="size-12 mx-auto text-muted-foreground" />
          <h2 className="text-xl font-semibold">{t("common.adminOnly")}</h2>
          <p className="text-muted-foreground">
            {t("common.adminOnlyMsg")}
          </p>
          <Link href="/admin/dashboard">
            <Button variant="outline">{t("common.backToDashboard")}</Button>
          </Link>
        </div>
      </div>
    );
  }

  return (
    <div className="space-y-6">
      <h1 className="text-2xl font-bold">{t("rbac.title")}</h1>

      <div className="grid gap-6 lg:grid-cols-[240px_1fr]">
        {/* Left: Roles list */}
        <Card className="h-fit">
          <CardHeader className="pb-3">
            <div className="flex items-center justify-between">
              <CardTitle className="text-base">{t("rbac.roles")}</CardTitle>
              <Button size="sm" onClick={openCreate}>
                <Plus className="size-4 mr-1" />
                {t("common.create")}
              </Button>
            </div>
          </CardHeader>
          <Separator />
          <CardContent className="p-2">
            {rolesQuery.isLoading ? (
              <p className="text-sm text-muted-foreground text-center py-8">
                {t("common.loading")}
              </p>
            ) : roles.length === 0 ? (
              <p className="text-sm text-muted-foreground text-center py-8">
                {t("rbac.noRoles")}
              </p>
            ) : (
              <div className="space-y-1">
                {roles.map((role) => {
                  const isActive = role.id === selectedRoleId;
                  return (
                    <div
                      key={role.id}
                      className={`flex items-center justify-between rounded-md px-3 py-2 cursor-pointer transition-colors ${
                        isActive
                          ? "bg-primary/10 text-primary"
                          : "hover:bg-muted"
                      }`}
                      onClick={() => setSelectedRoleId(role.id)}
                    >
                      <div className="min-w-0 flex-1">
                        <div className="flex items-center gap-2">
                          <span className="font-medium text-sm truncate">
                            {role.name}
                          </span>
                          {role.is_system && (
                              <Badge variant="secondary" className="text-[10px] px-1 py-0">
                                {t("rbac.systemRole")}
                            </Badge>
                          )}
                        </div>
                        {role.description && (
                          <p className="text-xs text-muted-foreground truncate">
                            {role.description}
                          </p>
                        )}
                      </div>
                      <DropdownMenu>
                        <DropdownMenuTrigger className="inline-flex items-center justify-center rounded-md p-1 hover:bg-muted transition-colors">
                          <MoreVertical className="size-4" />
                        </DropdownMenuTrigger>
                        <DropdownMenuContent align="end">
                          <DropdownMenuItem onClick={() => openEdit(role)}>
                            <Pencil className="size-4 mr-2" />
                            Edit
                          </DropdownMenuItem>
                          {!role.is_system && (
                            <DropdownMenuItem
                              className="text-destructive"
                              onClick={() => setDeleteRole(role)}
                            >
                              <Trash2 className="size-4 mr-2" />
                              Delete
                            </DropdownMenuItem>
                          )}
                        </DropdownMenuContent>
                      </DropdownMenu>
                    </div>
                  );
                })}
              </div>
            )}
          </CardContent>
          {totalPages > 1 && (
            <div className="flex items-center justify-center gap-1 border-t pt-2 pb-2 px-2">
              <Button
                variant="ghost"
                size="sm"
                disabled={page <= 1}
                onClick={() => setPage((p) => p - 1)}
              >
                {t("rbac.prev")}
              </Button>
              <span className="text-xs text-muted-foreground">
                {page}/{totalPages}
              </span>
              <Button
                variant="ghost"
                size="sm"
                disabled={page >= totalPages}
                onClick={() => setPage((p) => p + 1)}
              >
                {t("common.next")}
              </Button>
            </div>
          )}
        </Card>

        {/* Right: Permissions */}
        <Card>
          <CardHeader className="pb-3">
            <div className="flex items-center justify-between">
              <div>
                <CardTitle className="text-base flex items-center gap-2">
                  <Key className="size-4" />
                  {selectedRole ? (
                    <>
                      {selectedRole.name}
                      <Badge variant="outline" className="text-xs">
                        {permissions.length} permission{permissions.length !== 1 ? "s" : ""}
                      </Badge>
                    </>
                  ) : (
                    t("rbac.permissions")
                  )}
                </CardTitle>
                {selectedRole?.description && (
                  <p className="text-xs text-muted-foreground mt-1">
                    {selectedRole.description}
                  </p>
                )}
              </div>
              {selectedRoleId && (
                <Button size="sm" onClick={() => { resetPerm(); setAddPermOpen(true); }}>
                  <Plus className="size-4 mr-1" />
                  {t("rbac.addPermission")}
                </Button>
              )}
            </div>
          </CardHeader>
          <Separator />
          <CardContent className="p-0">
            {!selectedRoleId ? (
              <div className="flex flex-col items-center justify-center py-16 text-muted-foreground">
                <Shield className="size-10 mb-3" />
                <p className="text-sm">{t("rbac.selectRole")}</p>
              </div>
            ) : permsQuery.isLoading ? (
              <p className="text-sm text-muted-foreground text-center py-8">
                {t("common.loading")}
              </p>
            ) : permissions.length === 0 ? (
              <p className="text-sm text-muted-foreground text-center py-8">
                {t("rbac.noPermissions")}
              </p>
            ) : (
              <Table>
                <TableHeader>
                  <TableRow>
                    <TableHead>{t("rbac.action")}</TableHead>
                    <TableHead>{t("rbac.subject")}</TableHead>
                    <TableHead>{t("rbac.fieldsCol")}</TableHead>
                    <TableHead>{t("rbac.conditionsCol")}</TableHead>
                    <TableHead className="w-12" />
                  </TableRow>
                </TableHeader>
                <TableBody>
                  {permissions.map((p) => (
                    <TableRow key={p.id}>
                      <TableCell>
                        <code className="text-xs bg-muted px-1.5 py-0.5 rounded">
                          {p.action}
                        </code>
                      </TableCell>
                      <TableCell>
                        <code className="text-xs bg-muted px-1.5 py-0.5 rounded">
                          {p.subject}
                        </code>
                      </TableCell>
                      <TableCell>
                        {p.fields ? (
                          <div className="flex flex-wrap gap-1">
                            {p.fields.map((f) => (
                              <Badge key={f} variant="outline" className="text-xs">
                                {f}
                              </Badge>
                            ))}
                          </div>
                        ) : (
                          <span className="text-muted-foreground">&mdash;</span>
                        )}
                      </TableCell>
                      <TableCell>
                        {p.conditions ? (
                          <div className="flex flex-wrap gap-1">
                            {Object.entries(p.conditions).map(([k, v]) => (
                              <Badge key={k} variant="outline" className="text-xs">
                                {k}={v}
                              </Badge>
                            ))}
                          </div>
                        ) : (
                          <span className="text-muted-foreground">&mdash;</span>
                        )}
                      </TableCell>
                      <TableCell>
                        <DropdownMenu>
                          <DropdownMenuTrigger className="inline-flex items-center justify-center rounded-md p-1 hover:bg-muted transition-colors">
                            <MoreVertical className="size-4" />
                          </DropdownMenuTrigger>
                          <DropdownMenuContent align="end">
                            <DropdownMenuItem
                              className="text-destructive"
                              onClick={() => setRemovePerm(p)}
                            >
                              <Trash2 className="size-4 mr-2" />
                              Remove
                            </DropdownMenuItem>
                          </DropdownMenuContent>
                        </DropdownMenu>
                      </TableCell>
                    </TableRow>
                  ))}
                </TableBody>
              </Table>
            )}
          </CardContent>
        </Card>
      </div>

      {/* Create Role Dialog */}
      <Dialog open={createOpen} onOpenChange={setCreateOpen}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>{t("rbac.createRole")}</DialogTitle>
            <DialogDescription>{t("rbac.addRoleDesc")}</DialogDescription>
          </DialogHeader>
          <form
            onSubmit={submitRole((data) => createMutation.mutate(data))}
            className="space-y-4"
          >
            <div className="space-y-2">
              <Label htmlFor="name">{t("rbac.roleName")}</Label>
              <Input
                id="name"
                {...regRole("name")}
                placeholder="e.g. moderator"
              />
              {roleErrors.name && (
                <p className="text-xs text-destructive">
                  {roleErrors.name.message}
                </p>
              )}
            </div>
            <div className="space-y-2">
              <Label htmlFor="description">{t("rbac.roleDesc")}</Label>
              <Textarea
                id="description"
                {...regRole("description")}
                placeholder="Optional description"
                rows={2}
              />
            </div>
            <DialogFooter>
              <Button
                type="button"
                variant="outline"
                onClick={() => setCreateOpen(false)}
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

      {/* Edit Role Dialog */}
      <Dialog
        open={!!editRole}
        onOpenChange={(open) => !open && setEditRole(null)}
      >
        <DialogContent>
          <DialogHeader>
            <DialogTitle>{t("rbac.editRole")}</DialogTitle>
            <DialogDescription>
              Update &ldquo;{editRole?.name}&rdquo; role.
            </DialogDescription>
          </DialogHeader>
          <form
            onSubmit={submitRole((data) =>
              updateMutation.mutate({ id: editRole!.id, ...data }),
            )}
            className="space-y-4"
          >
            <div className="space-y-2">
              <Label htmlFor="edit-name">{t("rbac.roleName")}</Label>
              <Input id="edit-name" {...regRole("name")} />
              {roleErrors.name && (
                <p className="text-xs text-destructive">
                  {roleErrors.name.message}
                </p>
              )}
            </div>
            <div className="space-y-2">
              <Label htmlFor="edit-desc">{t("rbac.roleDesc")}</Label>
              <Textarea id="edit-desc" {...regRole("description")} rows={2} />
            </div>
            <DialogFooter>
              <Button
                type="button"
                variant="outline"
                onClick={() => setEditRole(null)}
              >
                {t("common.cancel")}
              </Button>
              <Button type="submit" disabled={updateMutation.isPending}>
                {updateMutation.isPending ? t("common.saving") : t("common.save")}
              </Button>
            </DialogFooter>
          </form>
        </DialogContent>
      </Dialog>

      {/* Delete Role Dialog */}
      <Dialog
        open={!!deleteRole}
        onOpenChange={(open) => !open && setDeleteRole(null)}
      >
        <DialogContent>
          <DialogHeader>
            <DialogTitle>{t("rbac.deleteRoleTitle")}</DialogTitle>
            <DialogDescription>
              {t("rbac.deleteRoleConfirm", { name: deleteRole?.name ?? "" })}
            </DialogDescription>
          </DialogHeader>
          <DialogFooter>
            <Button variant="outline" onClick={() => setDeleteRole(null)}>
              {t("common.cancel")}
            </Button>
            <Button
              variant="destructive"
              disabled={deleteMutation.isPending}
              onClick={() => deleteRole && deleteMutation.mutate(deleteRole.id)}
            >
              {deleteMutation.isPending ? t("common.saving") : t("common.delete")}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      {/* Add Permission Dialog */}
      <Dialog open={addPermOpen} onOpenChange={setAddPermOpen}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>{t("rbac.addPermissionTitle")}</DialogTitle>
            <DialogDescription>
              Grant a new permission to &ldquo;{selectedRole?.name}&rdquo;. Use *
              for wildcard.
            </DialogDescription>
          </DialogHeader>
          <form onSubmit={submitPerm(onSubmitPerm)} className="space-y-4">
            <div className="space-y-2">
              <Label>{t("rbac.action")}</Label>
              <Input
                {...regPerm("action")}
                placeholder="e.g. content-type::post.create"
                list="perm-action-list"
              />
              <datalist id="perm-action-list">
                {[...COMMON_ACTIONS, ...existingActions].map((a) => (
                  <option key={a} value={a} />
                ))}
              </datalist>
              {permErrors.action && (
                <p className="text-xs text-destructive">
                  {permErrors.action.message}
                </p>
              )}
            </div>
            <div className="space-y-2">
              <Label>{t("rbac.subject")}</Label>
              <Input
                {...regPerm("subject")}
                placeholder="e.g. content-type::post"
                list="perm-subject-list"
              />
              <datalist id="perm-subject-list">
                {[...COMMON_SUBJECTS, ...existingSubjects].map((s) => (
                  <option key={s} value={s} />
                ))}
              </datalist>
              {permErrors.subject && (
                <p className="text-xs text-destructive">
                  {permErrors.subject.message}
                </p>
              )}
            </div>
            <div className="space-y-2">
              <Label>{t("rbac.fieldsOptional")}</Label>
              <Input
                {...regPerm("fields")}
                placeholder="e.g. title,slug,content"
              />
            </div>
            <DialogFooter>
              <Button
                type="button"
                variant="outline"
                onClick={() => setAddPermOpen(false)}
              >
                {t("common.cancel")}
              </Button>
              <Button type="submit" disabled={addPermMutation.isPending}>
                {addPermMutation.isPending ? t("common.creating") : t("common.create")}
              </Button>
            </DialogFooter>
          </form>
        </DialogContent>
      </Dialog>

      {/* Remove Permission Dialog */}
      <Dialog
        open={!!removePerm}
        onOpenChange={(open) => !open && setRemovePerm(null)}
      >
        <DialogContent>
          <DialogHeader>
            <DialogTitle>{t("rbac.removePermissionTitle")}</DialogTitle>
            <DialogDescription>
              Remove{" "}
              <code className="bg-muted px-1 rounded">
                {removePerm?.action}
              </code>{" "}
              on{" "}
              <code className="bg-muted px-1 rounded">
                {removePerm?.subject}
              </code>
              ?
            </DialogDescription>
          </DialogHeader>
          <DialogFooter>
            <Button variant="outline" onClick={() => setRemovePerm(null)}>
              {t("common.cancel")}
            </Button>
            <Button
              variant="destructive"
              disabled={removePermMutation.isPending}
              onClick={() => removePermMutation.mutate()}
            >
              {removePermMutation.isPending ? t("common.saving") : t("common.delete")}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </div>
  );
}
