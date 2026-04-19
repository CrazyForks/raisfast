"use client";

import { useState } from "react";
import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import { Shield, User, Pencil, Trash2, Plus, MoreVertical } from "lucide-react";
import { useForm } from "react-hook-form";
import { z } from "zod";
import { zodResolver } from "@hookform/resolvers/zod";
import { toast } from "sonner";
import Link from "next/link";

import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { Card, CardContent } from "@/components/ui/card";
import { DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuTrigger } from "@/components/ui/dropdown-menu";
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
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { api, ApiError } from "@/lib/api";
import { useAuthStore } from "@/stores/auth";
import { useT } from "@/lib/i18n";

interface UserItem {
  id: string;
  email: string;
  username: string;
  role: string;
  avatar: string | null;
  bio: string | null;
  created_at: string;
}

interface PaginatedData<T> {
  items: T[];
  total: number;
  page: number;
  page_size: number;
}

function roleBadgeVariant(role: string) {
  switch (role) {
    case "admin":
      return "default" as const;
    case "author":
      return "secondary" as const;
    default:
      return "outline" as const;
  }
}

const roleSchema = z.object({
  role: z.enum(["reader", "author", "admin"]),
});

export default function UsersPage() {
  const { isAdmin, user: currentUser } = useAuthStore();
  const { t } = useT();
  const queryClient = useQueryClient();
  const [editUser, setEditUser] = useState<UserItem | null>(null);
  const [createOpen, setCreateOpen] = useState(false);
  const [newEmail, setNewEmail] = useState("");
  const [newUsername, setNewUsername] = useState("");
  const [newPassword, setNewPassword] = useState("");
  const [page, setPage] = useState(1);
  const pageSize = 20;

  const usersQuery = useQuery({
    queryKey: ["users", page],
    queryFn: () =>
      api.get<PaginatedData<UserItem>>(`/users?page=${page}&page_size=${pageSize}`),
  });

  type RoleForm = { role: string };

  const { handleSubmit, setValue, watch } = useForm<RoleForm>({
    resolver: zodResolver(roleSchema as never),
    defaultValues: { role: "reader" },
  });

  const roleValue = watch("role");

  const updateMutation = useMutation({
    mutationFn: ({ id, role }: { id: string; role: string }) =>
      api.put(`/users/${id}/role`, { role }),
    onSuccess: () => {
      toast.success(t("users.userRoleUpdated"));
      queryClient.invalidateQueries({ queryKey: ["users"] });
      setEditUser(null);
    },
    onError: (err) => {
      if (err instanceof ApiError) {
        toast.error(err.message);
      } else {
        toast.error(t("users.failedToUpdate"));
      }
    },
  });

  function openEdit(u: UserItem) {
    setEditUser(u);
    setValue("role", u.role);
  }

  const createMutation = useMutation({
    mutationFn: (data: { email: string; username: string; password: string }) =>
      api.post("/auth/register", data),
    onSuccess: () => {
      toast.success(t("users.userCreated"));
      queryClient.invalidateQueries({ queryKey: ["users"] });
      setCreateOpen(false);
      setNewEmail("");
      setNewUsername("");
      setNewPassword("");
    },
    onError: (err) => {
      if (err instanceof ApiError) {
        toast.error(err.message);
      } else {
        toast.error(t("users.failedToCreate"));
      }
    },
  });

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

  const users = usersQuery.data?.items ?? [];
  const totalPages = Math.ceil((usersQuery.data?.total ?? 0) / pageSize);

  return (
    <div className="space-y-6">
      <div className="flex items-center justify-between">
        <h1 className="text-2xl font-bold">{t("users.title")}</h1>
        <Dialog open={createOpen} onOpenChange={setCreateOpen}>
          <DialogTrigger render={<Button />}>
            <Plus className="size-4" />
            {t("users.newUser")}
          </DialogTrigger>
          <DialogContent>
            <DialogHeader>
              <DialogTitle>{t("users.createUser")}</DialogTitle>
              <DialogDescription>
                {t("users.registerNew")}
              </DialogDescription>
            </DialogHeader>
            <div className="space-y-4">
              <div className="space-y-2">
                <Label>{t("users.username")}</Label>
                <Input
                  value={newUsername}
                  onChange={(e) => setNewUsername(e.target.value)}
                  placeholder="username"
                />
              </div>
              <div className="space-y-2">
                <Label>{t("users.email")}</Label>
                <Input
                  type="email"
                  value={newEmail}
                  onChange={(e) => setNewEmail(e.target.value)}
                  placeholder="user@example.com"
                />
              </div>
              <div className="space-y-2">
                <Label>{t("users.password")}</Label>
                <Input
                  type="password"
                  value={newPassword}
                  onChange={(e) => setNewPassword(e.target.value)}
                  placeholder={t("users.minChars")}
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
                <Button
                  disabled={
                    createMutation.isPending ||
                    !newEmail ||
                    !newUsername ||
                    !newPassword
                  }
                  onClick={() =>
                    createMutation.mutate({
                      email: newEmail,
                      username: newUsername,
                      password: newPassword,
                    })
                  }
                >
                  {createMutation.isPending ? t("common.creating") : t("common.create")}
                </Button>
              </DialogFooter>
            </div>
          </DialogContent>
        </Dialog>
      </div>

      <Card>
        <CardContent className="p-0">
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead>{t("users.username")}</TableHead>
                <TableHead>{t("users.email")}</TableHead>
                <TableHead>{t("users.role")}</TableHead>
                <TableHead>{t("users.joined")}</TableHead>
                <TableHead className="text-right">{t("common.actions")}</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {usersQuery.isLoading ? (
                <TableRow>
                  <TableCell colSpan={5} className="text-center py-8">
                    {t("common.loading")}
                  </TableCell>
                </TableRow>
              ) : users.length === 0 ? (
                <TableRow>
                  <TableCell colSpan={5} className="text-center py-8">
                    {t("users.noUsers")}
                  </TableCell>
                </TableRow>
              ) : (
                users.map((u) => (
                  <TableRow key={u.id}>
                    <TableCell className="font-medium">
                      <div className="flex items-center gap-2">
                        <User className="size-4 text-muted-foreground" />
                        {u.username}
                      </div>
                    </TableCell>
                    <TableCell>{u.email}</TableCell>
                    <TableCell>
                      <Badge variant={roleBadgeVariant(u.role)}>
                        {u.role}
                      </Badge>
                    </TableCell>
                    <TableCell>
                      {new Date(u.created_at).toLocaleDateString()}
                    </TableCell>
                    <TableCell className="text-right">
                      {u.id !== currentUser?.id && (
                        <DropdownMenu>
                          <DropdownMenuTrigger
                            className="inline-flex items-center justify-center rounded-md p-1 hover:bg-muted transition-colors"
                          >
                            <MoreVertical className="size-4" />
                          </DropdownMenuTrigger>
                          <DropdownMenuContent align="end">
                            <DropdownMenuItem onClick={() => openEdit(u)}>
                              <Pencil className="size-4 mr-2" />
                              {t("users.editRole")}
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

      <Dialog open={!!editUser} onOpenChange={(open) => !open && setEditUser(null)}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>{t("users.editRole")}</DialogTitle>
            <DialogDescription>
              {t("users.changeRoleFor", { username: editUser?.username ?? "", email: editUser?.email ?? "" })}
            </DialogDescription>
          </DialogHeader>
          <form
            onSubmit={handleSubmit((data) =>
              updateMutation.mutate({ id: editUser!.id, role: data.role })
            )}
            className="space-y-4"
          >
            <div className="space-y-2">
              <Label>{t("users.role")}</Label>
              <Select
                value={roleValue}
                onValueChange={(val) => val && setValue("role", val)}
              >
                <SelectTrigger className="w-full">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value="reader">{t("users.reader")}</SelectItem>
                  <SelectItem value="author">{t("users.author")}</SelectItem>
                  <SelectItem value="admin">{t("users.admin")}</SelectItem>
                </SelectContent>
              </Select>
            </div>
            <DialogFooter>
              <Button
                type="button"
                variant="outline"
                onClick={() => setEditUser(null)}
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
    </div>
  );
}
