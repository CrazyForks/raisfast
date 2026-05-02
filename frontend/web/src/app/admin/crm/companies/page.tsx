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
import { crm, type Company } from "@/lib/crm";
import { useT } from "@/lib/i18n";

const INDUSTRIES = ["technology", "finance", "healthcare", "education", "retail", "manufacturing", "other"];
const SIZES = ["1-10", "11-50", "51-200", "201-500", "501-1000", "1000+"];

export default function CompaniesPage() {
  const { t } = useT();
  const router = useRouter();
  const queryClient = useQueryClient();
  const [page, setPage] = useState(1);
  const [createOpen, setCreateOpen] = useState(false);
  const [editOpen, setEditOpen] = useState(false);
  const [editing, setEditing] = useState<Company | null>(null);
  const [form, setForm] = useState({
    name: "",
    website: "",
    industry: "",
    size: "",
    phone: "",
    address: "",
    city: "",
    country: "",
    description: "",
  });

  const listQuery = useQuery({
    queryKey: ["crm-companies", page],
    queryFn: () => crm.listCompanies(page),
  });

  const deleteMutation = useMutation({
    mutationFn: (id: string) => crm.deleteCompany(id),
    onSuccess: () => {
      toast.success(t("common.deleted", { name: t("crm.company") }));
      queryClient.invalidateQueries({ queryKey: ["crm-companies"] });
    },
    onError: () => toast.error(t("common.failedToDelete", { name: t("crm.company") })),
  });

  const createMutation = useMutation({
    mutationFn: (data: Record<string, unknown>) => crm.createCompany(data),
    onSuccess: () => {
      toast.success(t("common.created", { name: t("crm.company") }));
      setCreateOpen(false);
      resetForm();
      queryClient.invalidateQueries({ queryKey: ["crm-companies"] });
    },
    onError: () => toast.error(t("common.failedToCreate", { name: t("crm.company") })),
  });

  const updateMutation = useMutation({
    mutationFn: ({ id, data }: { id: string; data: Record<string, unknown> }) =>
      crm.updateCompany(id, data),
    onSuccess: () => {
      toast.success(t("common.updated", { name: t("crm.company") }));
      setEditOpen(false);
      setEditing(null);
      queryClient.invalidateQueries({ queryKey: ["crm-companies"] });
    },
    onError: () => toast.error(t("common.failedToUpdate", { name: t("crm.company") })),
  });

  function resetForm() {
    setForm({ name: "", website: "", industry: "", size: "", phone: "", address: "", city: "", country: "", description: "" });
  }

  function openEdit(company: Company) {
    setEditing(company);
    setForm({
      name: company.name ?? "",
      website: company.website ?? "",
      industry: company.industry ?? "",
      size: company.size ?? "",
      phone: company.phone ?? "",
      address: company.address ?? "",
      city: company.city ?? "",
      country: company.country ?? "",
      description: company.description ?? "",
    });
    setEditOpen(true);
  }

  function handleSubmit(isEdit: boolean) {
    if (!form.name.trim()) {
      toast.error(t("common.nameRequired"));
      return;
    }
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
        <h1 className="text-2xl font-bold">{t("crm.companies")}</h1>
        <Button onClick={() => { resetForm(); setCreateOpen(true); }}>
          <Plus className="size-4" />
          {t("crm.newCompany")}
        </Button>
      </div>

      <Card>
        <CardContent className="p-0">
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead>{t("crm.name")}</TableHead>
                <TableHead>{t("crm.industry")}</TableHead>
                <TableHead>{t("crm.size")}</TableHead>
                <TableHead>{t("crm.phone")}</TableHead>
                <TableHead>{t("crm.city")}</TableHead>
                <TableHead>{t("crm.website")}</TableHead>
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
                  <TableCell colSpan={7} className="text-center py-8">{t("crm.noCompanies")}</TableCell>
                </TableRow>
              ) : (
                listQuery.data.items.map((company) => (
                  <TableRow key={company.id} className="cursor-pointer" onClick={() => router.push(`/admin/crm/companies/${company.id}`)}>
                    <TableCell className="font-medium">{company.name}</TableCell>
                    <TableCell>
                      {company.industry ? <Badge variant="secondary">{company.industry}</Badge> : "-"}
                    </TableCell>
                    <TableCell>{company.size ?? "-"}</TableCell>
                    <TableCell>{company.phone ?? "-"}</TableCell>
                    <TableCell>{company.city ?? "-"}</TableCell>
                    <TableCell>{company.website ?? "-"}</TableCell>
                    <TableCell className="text-right">
                      <DropdownMenu>
                        <DropdownMenuTrigger className="inline-flex items-center justify-center rounded-md p-1 hover:bg-muted transition-colors" onClick={(e) => { e.stopPropagation(); e.preventDefault(); }}>
                          <MoreVertical className="size-4" />
                        </DropdownMenuTrigger>
                        <DropdownMenuContent align="end">
                          <DropdownMenuItem onClick={(e) => { e.stopPropagation(); router.push(`/admin/crm/companies/${company.id}`); }}>
                            <Eye className="size-4 mr-2" />
                            {t("common.edit")}
                          </DropdownMenuItem>
                          <DropdownMenuItem onClick={(e) => { e.stopPropagation(); openEdit(company); }}>
                            <Pencil className="size-4 mr-2" />
                            {t("common.edit")}
                          </DropdownMenuItem>
                          <DropdownMenuItem className="text-destructive" onClick={(e) => { e.stopPropagation(); if (confirm(t("common.confirmDelete"))) deleteMutation.mutate(company.id); }} disabled={deleteMutation.isPending}>
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
          <Button variant="outline" size="sm" disabled={page <= 1} onClick={() => setPage((p) => p - 1)}>{t("common.previous")}</Button>
          <span className="text-sm text-muted-foreground">{t("common.pageOf", { page, total: totalPages })}</span>
          <Button variant="outline" size="sm" disabled={page >= totalPages} onClick={() => setPage((p) => p + 1)}>{t("common.next")}</Button>
        </div>
      )}

      <Dialog open={createOpen || editOpen} onOpenChange={(open) => { if (!open) { setCreateOpen(false); setEditOpen(false); setEditing(null); } }}>
        <DialogContent className="max-w-lg">
          <DialogHeader>
            <DialogTitle>{editOpen ? t("crm.editCompany") : t("crm.newCompany")}</DialogTitle>
          </DialogHeader>
          <div className="space-y-4">
            <div>
              <Label>{t("crm.companyName")}</Label>
              <Input value={form.name} onChange={(e) => setForm({ ...form, name: e.target.value })} />
            </div>
            <div className="grid grid-cols-2 gap-4">
              <div>
                <Label>{t("crm.industry")}</Label>
                <select className="flex h-9 w-full rounded-md border border-input bg-background px-3 py-1 text-sm" value={form.industry} onChange={(e) => setForm({ ...form, industry: e.target.value })}>
                  <option value="">{t("common.none")}</option>
                  {INDUSTRIES.map((s) => <option key={s} value={s}>{s}</option>)}
                </select>
              </div>
              <div>
                <Label>{t("crm.size")}</Label>
                <select className="flex h-9 w-full rounded-md border border-input bg-background px-3 py-1 text-sm" value={form.size} onChange={(e) => setForm({ ...form, size: e.target.value })}>
                  <option value="">{t("common.none")}</option>
                  {SIZES.map((s) => <option key={s} value={s}>{s}</option>)}
                </select>
              </div>
            </div>
            <div className="grid grid-cols-2 gap-4">
              <div>
                <Label>{t("crm.website")}</Label>
                <Input value={form.website} onChange={(e) => setForm({ ...form, website: e.target.value })} />
              </div>
              <div>
                <Label>{t("crm.phone")}</Label>
                <Input value={form.phone} onChange={(e) => setForm({ ...form, phone: e.target.value })} />
              </div>
            </div>
            <div className="grid grid-cols-2 gap-4">
              <div>
                <Label>{t("crm.city")}</Label>
                <Input value={form.city} onChange={(e) => setForm({ ...form, city: e.target.value })} />
              </div>
              <div>
                <Label>{t("crm.country")}</Label>
                <Input value={form.country} onChange={(e) => setForm({ ...form, country: e.target.value })} />
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
