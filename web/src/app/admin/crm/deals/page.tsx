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
import { crm, type Deal } from "@/lib/crm";
import { useT } from "@/lib/i18n";

const STAGES = ["prospecting", "qualification", "proposal", "negotiation", "closed_won", "closed_lost"];
const CURRENCIES = ["usd", "eur", "gbp", "cny", "jpy"];
const LOSS_REASONS = ["price", "competitor", "no_response", "no_budget", "no_need", "other"];

const STAGE_COLORS: Record<string, string> = {
  prospecting: "bg-blue-500",
  qualification: "bg-yellow-500",
  proposal: "bg-purple-500",
  negotiation: "bg-orange-500",
  closed_won: "bg-green-500",
  closed_lost: "bg-red-500",
};

function formatAmount(cents: number | undefined) {
  if (cents == null) return "-";
  return `$${(cents / 100).toLocaleString()}`;
}

export default function DealsPage() {
  const { t } = useT();
  const router = useRouter();
  const queryClient = useQueryClient();
  const [page, setPage] = useState(1);
  const [createOpen, setCreateOpen] = useState(false);
  const [editOpen, setEditOpen] = useState(false);
  const [editing, setEditing] = useState<Deal | null>(null);
  const [form, setForm] = useState({
    title: "",
    amount: "",
    currency: "usd",
    stage: "prospecting",
    probability: "50",
    description: "",
    close_date: "",
    loss_reason: "",
  });

  const listQuery = useQuery({
    queryKey: ["crm-deals", page],
    queryFn: () => crm.listDeals(page),
  });

  const deleteMutation = useMutation({
    mutationFn: (id: string) => crm.deleteDeal(id),
    onSuccess: () => {
      toast.success(t("common.deleted", { name: t("crm.deal") }));
      queryClient.invalidateQueries({ queryKey: ["crm-deals"] });
    },
    onError: () => toast.error(t("common.failedToDelete", { name: t("crm.deal") })),
  });

  const createMutation = useMutation({
    mutationFn: (data: Record<string, unknown>) => crm.createDeal(data),
    onSuccess: () => {
      toast.success(t("common.created", { name: t("crm.deal") }));
      setCreateOpen(false);
      resetForm();
      queryClient.invalidateQueries({ queryKey: ["crm-deals"] });
    },
    onError: () => toast.error(t("common.failedToCreate", { name: t("crm.deal") })),
  });

  const updateMutation = useMutation({
    mutationFn: ({ id, data }: { id: string; data: Record<string, unknown> }) =>
      crm.updateDeal(id, data),
    onSuccess: () => {
      toast.success(t("common.updated", { name: t("crm.deal") }));
      setEditOpen(false);
      setEditing(null);
      queryClient.invalidateQueries({ queryKey: ["crm-deals"] });
    },
    onError: () => toast.error(t("common.failedToUpdate", { name: t("crm.deal") })),
  });

  function resetForm() {
    setForm({ title: "", amount: "", currency: "usd", stage: "prospecting", probability: "50", description: "", close_date: "", loss_reason: "" });
  }

  function openEdit(deal: Deal) {
    setEditing(deal);
    setForm({
      title: deal.title ?? "",
      amount: deal.amount != null ? String(deal.amount / 100) : "",
      currency: deal.currency ?? "usd",
      stage: deal.stage ?? "prospecting",
      probability: deal.probability != null ? String(deal.probability) : "50",
      description: deal.description ?? "",
      close_date: deal.close_date ?? "",
      loss_reason: deal.loss_reason ?? "",
    });
    setEditOpen(true);
  }

  function handleSubmit(isEdit: boolean) {
    if (!form.title.trim()) {
      toast.error(t("crm.titleRequired"));
      return;
    }
    const data: Record<string, unknown> = {
      title: form.title,
      amount: form.amount ? Math.round(parseFloat(form.amount) * 100) : 0,
      currency: form.currency,
      stage: form.stage,
      probability: parseInt(form.probability) || 0,
      description: form.description,
      close_date: form.close_date || undefined,
      loss_reason: form.loss_reason || undefined,
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
        <h1 className="text-2xl font-bold">{t("crm.deals")}</h1>
        <Button onClick={() => { resetForm(); setCreateOpen(true); }}>
          <Plus className="size-4" />
          {t("crm.newDeal")}
        </Button>
      </div>

      <Card>
        <CardContent className="p-0">
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead>{t("crm.dealTitle")}</TableHead>
                <TableHead>{t("crm.amount")}</TableHead>
                <TableHead>{t("crm.stage")}</TableHead>
                <TableHead>{t("crm.probability")}</TableHead>
                <TableHead>{t("crm.closeDate")}</TableHead>
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
                  <TableCell colSpan={6} className="text-center py-8">{t("crm.noDeals")}</TableCell>
                </TableRow>
              ) : (
                listQuery.data.items.map((deal) => (
                  <TableRow key={deal.id} className="cursor-pointer" onClick={() => router.push(`/admin/crm/deals/${deal.id}`)}>
                    <TableCell className="font-medium">{deal.title}</TableCell>
                    <TableCell>{formatAmount(deal.amount)}</TableCell>
                    <TableCell>
                      <div className="flex items-center gap-1.5">
                        <div className={`size-2 rounded-full ${STAGE_COLORS[deal.stage] ?? "bg-gray-400"}`} />
                        <span className="text-sm">{(deal.stage ?? "").replace(/_/g, " ")}</span>
                      </div>
                    </TableCell>
                    <TableCell>{deal.probability ?? 0}%</TableCell>
                    <TableCell>{deal.close_date ?? "-"}</TableCell>
                    <TableCell className="text-right">
                      <DropdownMenu>
                        <DropdownMenuTrigger className="inline-flex items-center justify-center rounded-md p-1 hover:bg-muted transition-colors" onClick={(e) => { e.stopPropagation(); e.preventDefault(); }}>
                          <MoreVertical className="size-4" />
                        </DropdownMenuTrigger>
                        <DropdownMenuContent align="end">
                          <DropdownMenuItem onClick={(e) => { e.stopPropagation(); router.push(`/admin/crm/deals/${deal.id}`); }}>
                            <Eye className="size-4 mr-2" />{t("common.edit")}
                          </DropdownMenuItem>
                          <DropdownMenuItem onClick={(e) => { e.stopPropagation(); openEdit(deal); }}>
                            <Pencil className="size-4 mr-2" />{t("common.edit")}
                          </DropdownMenuItem>
                          <DropdownMenuItem className="text-destructive" onClick={(e) => { e.stopPropagation(); if (confirm(t("common.confirmDelete"))) deleteMutation.mutate(deal.id); }} disabled={deleteMutation.isPending}>
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
        <DialogContent className="max-w-lg">
          <DialogHeader>
            <DialogTitle>{editOpen ? t("crm.editDeal") : t("crm.newDeal")}</DialogTitle>
          </DialogHeader>
          <div className="space-y-4">
            <div>
              <Label>{t("crm.dealTitle")}</Label>
              <Input value={form.title} onChange={(e) => setForm({ ...form, title: e.target.value })} />
            </div>
            <div className="grid grid-cols-2 gap-4">
              <div>
                <Label>{t("crm.amount")}</Label>
                <Input type="number" value={form.amount} onChange={(e) => setForm({ ...form, amount: e.target.value })} placeholder="0.00" />
              </div>
              <div>
                <Label>{t("crm.stage")}</Label>
                <select className="flex h-9 w-full rounded-md border border-input bg-background px-3 py-1 text-sm" value={form.stage} onChange={(e) => setForm({ ...form, stage: e.target.value })}>
                  {STAGES.map((s) => <option key={s} value={s}>{s.replace(/_/g, " ")}</option>)}
                </select>
              </div>
            </div>
            <div className="grid grid-cols-2 gap-4">
              <div>
                <Label>{t("crm.probability")}</Label>
                <Input type="number" min={0} max={100} value={form.probability} onChange={(e) => setForm({ ...form, probability: e.target.value })} />
              </div>
              <div>
                <Label>{t("crm.closeDate")}</Label>
                <Input type="date" value={form.close_date} onChange={(e) => setForm({ ...form, close_date: e.target.value })} />
              </div>
            </div>
            <div>
              <Label>{t("common.description")}</Label>
              <Input value={form.description} onChange={(e) => setForm({ ...form, description: e.target.value })} />
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
