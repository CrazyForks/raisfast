"use client";

import { useState } from "react";
import { useRouter } from "next/navigation";
import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import { Plus, Trash2, MoreVertical, Pencil, Eye } from "lucide-react";
import { toast } from "sonner";

import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import { Skeleton } from "@/components/ui/skeleton";
import { Badge } from "@/components/ui/badge";
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogFooter,
} from "@/components/ui/dialog";
import { DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuTrigger } from "@/components/ui/dropdown-menu";
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
import { crm, type Contact } from "@/lib/crm";
import { useT } from "@/lib/i18n";

const SOURCES = ["website", "referral", "social_media", "email", "event", "advertising", "other"];
const STATUSES = ["new", "engaged", "qualified", "unqualified"];
const LIFECYCLE_STAGES = [
  "subscriber", "lead", "marketing_qualified_lead",
  "sales_qualified_lead", "opportunity", "customer", "evangelist",
];

export default function ContactsPage() {
  const { t } = useT();
  const router = useRouter();
  const queryClient = useQueryClient();
  const [page, setPage] = useState(1);
  const [createOpen, setCreateOpen] = useState(false);
  const [editOpen, setEditOpen] = useState(false);
  const [editing, setEditing] = useState<Contact | null>(null);
  const [form, setForm] = useState({
    first_name: "",
    last_name: "",
    email: "",
    phone: "",
    job_title: "",
    company: "",
    source: "",
    status: "new",
    lifecycle_stage: "lead",
    notes: "",
  });

  const listQuery = useQuery({
    queryKey: ["crm-contacts", page],
    queryFn: () => crm.listContacts(page),
  });

  const deleteMutation = useMutation({
    mutationFn: (id: string) => crm.deleteContact(id),
    onSuccess: () => {
      toast.success(t("common.deleted", { name: t("crm.contact") }));
      queryClient.invalidateQueries({ queryKey: ["crm-contacts"] });
    },
    onError: () => toast.error(t("common.failedToDelete", { name: t("crm.contact") })),
  });

  const createMutation = useMutation({
    mutationFn: (data: Record<string, unknown>) => crm.createContact(data),
    onSuccess: () => {
      toast.success(t("common.created", { name: t("crm.contact") }));
      setCreateOpen(false);
      resetForm();
      queryClient.invalidateQueries({ queryKey: ["crm-contacts"] });
    },
    onError: () => toast.error(t("common.failedToCreate", { name: t("crm.contact") })),
  });

  const updateMutation = useMutation({
    mutationFn: ({ id, data }: { id: string; data: Record<string, unknown> }) =>
      crm.updateContact(id, data),
    onSuccess: () => {
      toast.success(t("common.updated", { name: t("crm.contact") }));
      setEditOpen(false);
      setEditing(null);
      queryClient.invalidateQueries({ queryKey: ["crm-contacts"] });
    },
    onError: () => toast.error(t("common.failedToUpdate", { name: t("crm.contact") })),
  });

  function resetForm() {
    setForm({
      first_name: "", last_name: "", email: "", phone: "",
      job_title: "", company: "", source: "", status: "new",
      lifecycle_stage: "lead", notes: "",
    });
  }

  function openEdit(contact: Contact) {
    setEditing(contact);
    setForm({
      first_name: contact.first_name ?? "",
      last_name: contact.last_name ?? "",
      email: contact.email ?? "",
      phone: contact.phone ?? "",
      job_title: contact.job_title ?? "",
      company: contact.company ?? "",
      source: contact.source ?? "",
      status: contact.status ?? "new",
      lifecycle_stage: contact.lifecycle_stage ?? "lead",
      notes: contact.notes ?? "",
    });
    setEditOpen(true);
  }

  function handleSubmit(isEdit: boolean) {
    const data = { ...form };
    if (isEdit && editing) {
      updateMutation.mutate({ id: editing.id, data });
    } else {
      createMutation.mutate(data);
    }
  }

  const totalPages = Math.ceil((listQuery.data?.total ?? 0) / 50);

  return (
    <div className="space-y-6">
      <div className="flex items-center justify-between">
        <h1 className="text-2xl font-bold">{t("crm.contacts")}</h1>
        <Button onClick={() => { resetForm(); setCreateOpen(true); }}>
          <Plus className="size-4" />
          {t("crm.newContact")}
        </Button>
      </div>

      <Card>
        <CardContent className="p-0">
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead>{t("crm.name")}</TableHead>
                <TableHead>{t("crm.email")}</TableHead>
                <TableHead>{t("crm.phone")}</TableHead>
                <TableHead>{t("crm.company")}</TableHead>
                <TableHead>{t("crm.source")}</TableHead>
                <TableHead>{t("crm.lifecycleStage")}</TableHead>
                <TableHead className="text-right">{t("common.actions")}</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {listQuery.isLoading ? (
                <TableRow>
                  <TableCell colSpan={7} className="text-center py-8">{t("common.loading")}</TableCell>
                </TableRow>
              ) : !listQuery.data?.items?.length ? (
                <TableRow>
                  <TableCell colSpan={7} className="text-center py-8">{t("crm.noContacts")}</TableCell>
                </TableRow>
              ) : (
                listQuery.data.items.map((contact) => (
                  <TableRow key={contact.id} className="cursor-pointer" onClick={() => router.push(`/admin/crm/contacts/${contact.id}`)}>
                    <TableCell className="font-medium">
                      {contact.first_name} {contact.last_name}
                    </TableCell>
                    <TableCell>{contact.email ?? "-"}</TableCell>
                    <TableCell>{contact.phone ?? "-"}</TableCell>
                    <TableCell>{contact.company ?? "-"}</TableCell>
                    <TableCell>
                      {contact.source ? (
                        <Badge variant="secondary">{contact.source.replace(/_/g, " ")}</Badge>
                      ) : "-"}
                    </TableCell>
                    <TableCell>
                      {contact.lifecycle_stage ? (
                        <Badge variant="outline">{contact.lifecycle_stage.replace(/_/g, " ")}</Badge>
                      ) : "-"}
                    </TableCell>
                    <TableCell className="text-right">
                      <DropdownMenu>
                        <DropdownMenuTrigger className="inline-flex items-center justify-center rounded-md p-1 hover:bg-muted transition-colors" onClick={(e) => { e.stopPropagation(); e.preventDefault(); }}>
                          <MoreVertical className="size-4" />
                        </DropdownMenuTrigger>
                        <DropdownMenuContent align="end">
                          <DropdownMenuItem onClick={(e) => { e.stopPropagation(); router.push(`/admin/crm/contacts/${contact.id}`); }}>
                            <Eye className="size-4 mr-2" />
                            {t("common.edit")}
                          </DropdownMenuItem>
                          <DropdownMenuItem onClick={(e) => { e.stopPropagation(); openEdit(contact); }}>
                            <Pencil className="size-4 mr-2" />
                            {t("common.edit")}
                          </DropdownMenuItem>
                          <DropdownMenuItem className="text-destructive" onClick={(e) => { e.stopPropagation(); if (confirm(t("common.confirmDelete"))) deleteMutation.mutate(contact.id); }} disabled={deleteMutation.isPending}>
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
          <Button variant="outline" size="sm" disabled={page <= 1} onClick={() => setPage((p) => p - 1)}>
            {t("common.previous")}
          </Button>
          <span className="text-sm text-muted-foreground">
            {t("common.pageOf", { page, total: totalPages })}
          </span>
          <Button variant="outline" size="sm" disabled={page >= totalPages} onClick={() => setPage((p) => p + 1)}>
            {t("common.next")}
          </Button>
        </div>
      )}

      <Dialog open={createOpen || editOpen} onOpenChange={(open) => { if (!open) { setCreateOpen(false); setEditOpen(false); setEditing(null); } }}>
        <DialogContent className="max-w-lg">
          <DialogHeader>
            <DialogTitle>{editOpen ? t("crm.editContact") : t("crm.newContact")}</DialogTitle>
          </DialogHeader>
          <div className="space-y-4">
            <div className="grid grid-cols-2 gap-4">
              <div>
                <Label>{t("crm.firstName")}</Label>
                <Input value={form.first_name} onChange={(e) => setForm({ ...form, first_name: e.target.value })} />
              </div>
              <div>
                <Label>{t("crm.lastName")}</Label>
                <Input value={form.last_name} onChange={(e) => setForm({ ...form, last_name: e.target.value })} />
              </div>
            </div>
            <div className="grid grid-cols-2 gap-4">
              <div>
                <Label>{t("crm.email")}</Label>
                <Input type="email" value={form.email} onChange={(e) => setForm({ ...form, email: e.target.value })} />
              </div>
              <div>
                <Label>{t("crm.phone")}</Label>
                <Input value={form.phone} onChange={(e) => setForm({ ...form, phone: e.target.value })} />
              </div>
            </div>
            <div className="grid grid-cols-2 gap-4">
              <div>
                <Label>{t("crm.jobTitle")}</Label>
                <Input value={form.job_title} onChange={(e) => setForm({ ...form, job_title: e.target.value })} />
              </div>
              <div>
                <Label>{t("crm.company")}</Label>
                <Input value={form.company} onChange={(e) => setForm({ ...form, company: e.target.value })} />
              </div>
            </div>
            <div className="grid grid-cols-2 gap-4">
              <div>
                <Label>{t("crm.source")}</Label>
                <select className="flex h-9 w-full rounded-md border border-input bg-background px-3 py-1 text-sm" value={form.source} onChange={(e) => setForm({ ...form, source: e.target.value })}>
                  <option value="">{t("common.none")}</option>
                  {SOURCES.map((s) => <option key={s} value={s}>{s.replace(/_/g, " ")}</option>)}
                </select>
              </div>
              <div>
                <Label>{t("crm.lifecycleStage")}</Label>
                <select className="flex h-9 w-full rounded-md border border-input bg-background px-3 py-1 text-sm" value={form.lifecycle_stage} onChange={(e) => setForm({ ...form, lifecycle_stage: e.target.value })}>
                  {LIFECYCLE_STAGES.map((s) => <option key={s} value={s}>{s.replace(/_/g, " ")}</option>)}
                </select>
              </div>
            </div>
            <div>
              <Label>{t("common.description")}</Label>
              <Input value={form.notes} onChange={(e) => setForm({ ...form, notes: e.target.value })} />
            </div>
          </div>
          <DialogFooter>
            <Button variant="outline" onClick={() => { setCreateOpen(false); setEditOpen(false); setEditing(null); }}>
              {t("common.cancel")}
            </Button>
            <Button onClick={() => handleSubmit(editOpen)} disabled={createMutation.isPending || updateMutation.isPending}>
              {createMutation.isPending || updateMutation.isPending ? t("common.saving") : (editOpen ? t("common.save") : t("common.create"))}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </div>
  );
}
