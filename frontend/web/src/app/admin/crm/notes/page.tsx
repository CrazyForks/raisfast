"use client";

import { useState } from "react";
import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import { Plus, Trash2, MoreVertical, Pencil } from "lucide-react";
import { toast } from "sonner";

import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
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
import { Badge } from "@/components/ui/badge";
import { crm, type Note } from "@/lib/crm";
import { useT } from "@/lib/i18n";

export default function NotesPage() {
  const { t } = useT();
  const queryClient = useQueryClient();
  const [page, setPage] = useState(1);
  const [createOpen, setCreateOpen] = useState(false);
  const [editOpen, setEditOpen] = useState(false);
  const [editing, setEditing] = useState<Note | null>(null);
  const [form, setForm] = useState({ content: "", contact: "", company: "", deal: "" });

  const listQuery = useQuery({
    queryKey: ["crm-notes", page],
    queryFn: () => crm.listNotes(page),
  });

  const deleteMutation = useMutation({
    mutationFn: (id: string) => crm.deleteNote(id),
    onSuccess: () => {
      toast.success(t("common.deleted", { name: t("crm.note") }));
      queryClient.invalidateQueries({ queryKey: ["crm-notes"] });
    },
    onError: () => toast.error(t("common.failedToDelete", { name: t("crm.note") })),
  });

  const createMutation = useMutation({
    mutationFn: (data: Record<string, unknown>) => crm.createNote(data),
    onSuccess: () => {
      toast.success(t("common.created", { name: t("crm.note") }));
      setCreateOpen(false);
      setForm({ content: "", contact: "", company: "", deal: "" });
      queryClient.invalidateQueries({ queryKey: ["crm-notes"] });
    },
    onError: () => toast.error(t("common.failedToCreate", { name: t("crm.note") })),
  });

  const updateMutation = useMutation({
    mutationFn: ({ id, data }: { id: string; data: Record<string, unknown> }) =>
      crm.updateNote(id, data),
    onSuccess: () => {
      toast.success(t("common.updated", { name: t("crm.note") }));
      setEditOpen(false);
      setEditing(null);
      queryClient.invalidateQueries({ queryKey: ["crm-notes"] });
    },
    onError: () => toast.error(t("common.failedToUpdate", { name: t("crm.note") })),
  });

  function openEdit(note: Note) {
    setEditing(note);
    setForm({
      content: note.content ?? "",
      contact: note.contact ?? "",
      company: note.company ?? "",
      deal: note.deal ?? "",
    });
    setEditOpen(true);
  }

  function handleSubmit(isEdit: boolean) {
    if (!form.content.trim()) {
      toast.error(t("crm.contentRequired"));
      return;
    }
    const data: Record<string, unknown> = { content: form.content };
    if (form.contact) data.contact = form.contact;
    if (form.company) data.company = form.company;
    if (form.deal) data.deal = form.deal;
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
        <h1 className="text-2xl font-bold">{t("crm.notes")}</h1>
        <Button onClick={() => { setForm({ content: "", contact: "", company: "", deal: "" }); setCreateOpen(true); }}>
          <Plus className="size-4" />
          {t("crm.newNote")}
        </Button>
      </div>

      <Card>
        <CardContent className="p-0">
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead>{t("crm.content")}</TableHead>
                <TableHead>{t("crm.relatedTo")}</TableHead>
                <TableHead>{t("crm.pinned")}</TableHead>
                <TableHead>{t("crm.createdAt")}</TableHead>
                <TableHead className="text-right">{t("common.actions")}</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {listQuery.isLoading ? (
                <TableRow>
                  <TableCell colSpan={5} className="text-center py-8">{t("common.loading")}</TableCell>
                </TableRow>
              ) : !listQuery.data?.items?.length ? (
                <TableRow>
                  <TableCell colSpan={5} className="text-center py-8">{t("crm.noNotes")}</TableCell>
                </TableRow>
              ) : (
                listQuery.data.items.map((note) => (
                  <TableRow key={note.id}>
                    <TableCell className="max-w-xs truncate">{note.content}</TableCell>
                    <TableCell>
                      {note.contact && <Badge variant="secondary" className="mr-1">Contact</Badge>}
                      {note.company && <Badge variant="secondary" className="mr-1">Company</Badge>}
                      {note.deal && <Badge variant="secondary">Deal</Badge>}
                    </TableCell>
                    <TableCell>{note.pinned === 1 ? t("field.yes") : t("field.no")}</TableCell>
                    <TableCell>{new Date(note.created_at).toLocaleDateString()}</TableCell>
                    <TableCell className="text-right">
                      <DropdownMenu>
                        <DropdownMenuTrigger className="inline-flex items-center justify-center rounded-md p-1 hover:bg-muted transition-colors">
                          <MoreVertical className="size-4" />
                        </DropdownMenuTrigger>
                        <DropdownMenuContent align="end">
                          <DropdownMenuItem onClick={() => openEdit(note)}>
                            <Pencil className="size-4 mr-2" />{t("common.edit")}
                          </DropdownMenuItem>
                          <DropdownMenuItem className="text-destructive" onClick={() => { if (confirm(t("common.confirmDelete"))) deleteMutation.mutate(note.id); }} disabled={deleteMutation.isPending}>
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

      {totalPages > 1 && (
        <div className="flex items-center justify-center gap-2">
          <Button variant="outline" size="sm" disabled={page <= 1} onClick={() => setPage((p) => p - 1)}>{t("common.previous")}</Button>
          <span className="text-sm text-muted-foreground">{t("common.pageOf", { page, total: totalPages })}</span>
          <Button variant="outline" size="sm" disabled={page >= totalPages} onClick={() => setPage((p) => p + 1)}>{t("common.next")}</Button>
        </div>
      )}

      <Dialog open={createOpen || editOpen} onOpenChange={(open) => { if (!open) { setCreateOpen(false); setEditOpen(false); setEditing(null); } }}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>{editOpen ? t("crm.editNote") : t("crm.newNote")}</DialogTitle>
          </DialogHeader>
          <div className="space-y-4">
            <div>
              <Label>{t("crm.content")}</Label>
              <Input value={form.content} onChange={(e) => setForm({ ...form, content: e.target.value })} />
            </div>
            <div>
              <Label>{t("crm.relatedContactId")}</Label>
              <Input value={form.contact} onChange={(e) => setForm({ ...form, contact: e.target.value })} placeholder={t("field.relatedItemId")} />
            </div>
            <div>
              <Label>{t("crm.relatedCompanyId")}</Label>
              <Input value={form.company} onChange={(e) => setForm({ ...form, company: e.target.value })} placeholder={t("field.relatedItemId")} />
            </div>
            <div>
              <Label>{t("crm.relatedDealId")}</Label>
              <Input value={form.deal} onChange={(e) => setForm({ ...form, deal: e.target.value })} placeholder={t("field.relatedItemId")} />
            </div>
          </div>
          <DialogFooter>
            <Button variant="outline" onClick={() => { setCreateOpen(false); setEditOpen(false); setEditing(null); }}>{t("common.cancel")}</Button>
            <Button onClick={() => handleSubmit(editOpen)} disabled={createMutation.isPending || updateMutation.isPending}>
              {createMutation.isPending || updateMutation.isPending ? t("common.saving") : (editOpen ? t("common.save") : t("common.create"))}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </div>
  );
}
