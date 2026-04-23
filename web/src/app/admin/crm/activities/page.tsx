"use client";

import { useState } from "react";
import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import { Plus, Trash2, MoreVertical, Pencil } from "lucide-react";
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
import { crm, type Activity } from "@/lib/crm";
import { useT } from "@/lib/i18n";

const TYPES = ["call", "email", "meeting", "task", "note", "demo", "follow_up"];
const OUTCOMES = ["completed", "rescheduled", "cancelled", "no_show"];

export default function ActivitiesPage() {
  const { t } = useT();
  const queryClient = useQueryClient();
  const [page, setPage] = useState(1);
  const [createOpen, setCreateOpen] = useState(false);
  const [editOpen, setEditOpen] = useState(false);
  const [editing, setEditing] = useState<Activity | null>(null);
  const [form, setForm] = useState({
    type: "call",
    subject: "",
    content: "",
    activity_date: "",
    duration_minutes: "",
    outcome: "",
  });

  const listQuery = useQuery({
    queryKey: ["crm-activities", page],
    queryFn: () => crm.listActivities(page),
  });

  const deleteMutation = useMutation({
    mutationFn: (id: string) => crm.deleteActivity(id),
    onSuccess: () => {
      toast.success(t("common.deleted", { name: t("crm.activity") }));
      queryClient.invalidateQueries({ queryKey: ["crm-activities"] });
    },
    onError: () => toast.error(t("common.failedToDelete", { name: t("crm.activity") })),
  });

  const createMutation = useMutation({
    mutationFn: (data: Record<string, unknown>) => crm.createActivity(data),
    onSuccess: () => {
      toast.success(t("common.created", { name: t("crm.activity") }));
      setCreateOpen(false);
      resetForm();
      queryClient.invalidateQueries({ queryKey: ["crm-activities"] });
    },
    onError: () => toast.error(t("common.failedToCreate", { name: t("crm.activity") })),
  });

  const updateMutation = useMutation({
    mutationFn: ({ id, data }: { id: string; data: Record<string, unknown> }) =>
      crm.updateActivity(id, data),
    onSuccess: () => {
      toast.success(t("common.updated", { name: t("crm.activity") }));
      setEditOpen(false);
      setEditing(null);
      queryClient.invalidateQueries({ queryKey: ["crm-activities"] });
    },
    onError: () => toast.error(t("common.failedToUpdate", { name: t("crm.activity") })),
  });

  function resetForm() {
    setForm({ type: "call", subject: "", content: "", activity_date: "", duration_minutes: "", outcome: "" });
  }

  function openEdit(activity: Activity) {
    setEditing(activity);
    setForm({
      type: activity.type ?? "call",
      subject: activity.subject ?? "",
      content: activity.content ?? "",
      activity_date: activity.activity_date ?? "",
      duration_minutes: activity.duration_minutes != null ? String(activity.duration_minutes) : "",
      outcome: activity.outcome ?? "",
    });
    setEditOpen(true);
  }

  function handleSubmit(isEdit: boolean) {
    const data: Record<string, unknown> = {
      type: form.type,
      subject: form.subject,
      content: form.content,
      activity_date: form.activity_date || undefined,
      duration_minutes: form.duration_minutes ? parseInt(form.duration_minutes) : undefined,
      outcome: form.outcome || undefined,
    };
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
        <h1 className="text-2xl font-bold">{t("crm.activities")}</h1>
        <Button onClick={() => { resetForm(); setCreateOpen(true); }}>
          <Plus className="size-4" />
          {t("crm.newActivity")}
        </Button>
      </div>

      <Card>
        <CardContent className="p-0">
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead>{t("crm.type")}</TableHead>
                <TableHead>{t("crm.subject")}</TableHead>
                <TableHead>{t("crm.date")}</TableHead>
                <TableHead>{t("crm.duration")}</TableHead>
                <TableHead>{t("crm.outcome")}</TableHead>
                <TableHead className="text-right">{t("common.actions")}</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {listQuery.isLoading ? (
                <TableRow>
                  <TableCell colSpan={6} className="text-center py-8">{t("common.loading")}</TableCell>
                </TableRow>
              ) : !listQuery.data?.items?.length ? (
                <TableRow>
                  <TableCell colSpan={6} className="text-center py-8">{t("crm.noActivities")}</TableCell>
                </TableRow>
              ) : (
                listQuery.data.items.map((activity) => (
                  <TableRow key={activity.id}>
                    <TableCell><Badge variant="secondary">{(activity.type ?? "").replace(/_/g, " ")}</Badge></TableCell>
                    <TableCell className="font-medium">{activity.subject ?? "-"}</TableCell>
                    <TableCell>{activity.activity_date ?? "-"}</TableCell>
                    <TableCell>{activity.duration_minutes != null ? `${activity.duration_minutes}min` : "-"}</TableCell>
                    <TableCell>{activity.outcome ? <Badge variant="outline">{activity.outcome.replace(/_/g, " ")}</Badge> : "-"}</TableCell>
                    <TableCell className="text-right">
                      <DropdownMenu>
                        <DropdownMenuTrigger className="inline-flex items-center justify-center rounded-md p-1 hover:bg-muted transition-colors">
                          <MoreVertical className="size-4" />
                        </DropdownMenuTrigger>
                        <DropdownMenuContent align="end">
                          <DropdownMenuItem onClick={() => openEdit(activity)}>
                            <Pencil className="size-4 mr-2" />{t("common.edit")}
                          </DropdownMenuItem>
                          <DropdownMenuItem className="text-destructive" onClick={() => { if (confirm(t("common.confirmDelete"))) deleteMutation.mutate(activity.id); }} disabled={deleteMutation.isPending}>
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
            <DialogTitle>{editOpen ? t("crm.editActivity") : t("crm.newActivity")}</DialogTitle>
          </DialogHeader>
          <div className="space-y-4">
            <div className="grid grid-cols-2 gap-4">
              <div>
                <Label>{t("crm.type")}</Label>
                <select className="flex h-9 w-full rounded-md border border-input bg-background px-3 py-1 text-sm" value={form.type} onChange={(e) => setForm({ ...form, type: e.target.value })}>
                  {TYPES.map((s) => <option key={s} value={s}>{s.replace(/_/g, " ")}</option>)}
                </select>
              </div>
              <div>
                <Label>{t("crm.outcome")}</Label>
                <select className="flex h-9 w-full rounded-md border border-input bg-background px-3 py-1 text-sm" value={form.outcome} onChange={(e) => setForm({ ...form, outcome: e.target.value })}>
                  <option value="">{t("common.none")}</option>
                  {OUTCOMES.map((s) => <option key={s} value={s}>{s.replace(/_/g, " ")}</option>)}
                </select>
              </div>
            </div>
            <div>
              <Label>{t("crm.subject")}</Label>
              <Input value={form.subject} onChange={(e) => setForm({ ...form, subject: e.target.value })} />
            </div>
            <div>
              <Label>{t("crm.content")}</Label>
              <Input value={form.content} onChange={(e) => setForm({ ...form, content: e.target.value })} />
            </div>
            <div className="grid grid-cols-2 gap-4">
              <div>
                <Label>{t("crm.date")}</Label>
                <Input type="date" value={form.activity_date} onChange={(e) => setForm({ ...form, activity_date: e.target.value })} />
              </div>
              <div>
                <Label>{t("crm.duration")} (min)</Label>
                <Input type="number" value={form.duration_minutes} onChange={(e) => setForm({ ...form, duration_minutes: e.target.value })} />
              </div>
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
