"use client";

import { useState } from "react";
import { useRouter } from "next/navigation";
import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import { Plus, ArrowRight, DollarSign } from "lucide-react";
import { toast } from "sonner";

import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Skeleton } from "@/components/ui/skeleton";
import { Badge } from "@/components/ui/badge";
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogFooter,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { crm, type PipelineStage } from "@/lib/crm";
import { useT } from "@/lib/i18n";

const STAGE_ORDER = [
  "prospecting",
  "qualification",
  "proposal",
  "negotiation",
  "closed_won",
  "closed_lost",
];

const STAGE_COLORS: Record<string, string> = {
  prospecting: "bg-blue-500",
  qualification: "bg-yellow-500",
  proposal: "bg-purple-500",
  negotiation: "bg-orange-500",
  closed_won: "bg-green-500",
  closed_lost: "bg-red-500",
};

function formatAmount(cents: number) {
  return `$${(cents / 100).toLocaleString()}`;
}

export default function PipelinePage() {
  const { t } = useT();
  const router = useRouter();
  const queryClient = useQueryClient();
  const [createOpen, setCreateOpen] = useState(false);
  const [form, setForm] = useState({
    title: "",
    amount: "",
    stage: "prospecting",
    probability: "50",
    description: "",
  });

  const pipelineQuery = useQuery({
    queryKey: ["crm-pipeline"],
    queryFn: crm.getPipeline,
  });

  const contactsQuery = useQuery({
    queryKey: ["crm-contacts-all"],
    queryFn: () => crm.listContacts(1, 100),
  });

  const companiesQuery = useQuery({
    queryKey: ["crm-companies-all"],
    queryFn: () => crm.listCompanies(1, 100),
  });

  const createMutation = useMutation({
    mutationFn: (data: Record<string, unknown>) => crm.createDeal(data),
    onSuccess: () => {
      toast.success(t("crm.dealCreated"));
      setCreateOpen(false);
      setForm({ title: "", amount: "", stage: "prospecting", probability: "50", description: "" });
      queryClient.invalidateQueries({ queryKey: ["crm-pipeline"] });
    },
    onError: () => toast.error(t("crm.failedToCreateDeal")),
  });

  const advanceMutation = useMutation({
    mutationFn: ({ dealId, stage }: { dealId: string; stage: string }) =>
      crm.advanceDealStage(dealId, stage),
    onSuccess: () => {
      toast.success(t("crm.dealStageUpdated"));
      queryClient.invalidateQueries({ queryKey: ["crm-pipeline"] });
    },
    onError: () => toast.error(t("crm.failedToUpdateStage")),
  });

  function handleAdvance(dealId: string, currentStage: string) {
    const idx = STAGE_ORDER.indexOf(currentStage);
    if (idx < 0 || idx >= STAGE_ORDER.length - 1) return;
    const next = STAGE_ORDER[idx + 1];
    advanceMutation.mutate({ dealId, stage: next });
  }

  function handleCreate() {
    if (!form.title.trim()) {
      toast.error(t("crm.titleRequired"));
      return;
    }
    createMutation.mutate({
      title: form.title,
      amount: form.amount ? Math.round(parseFloat(form.amount) * 100) : 0,
      currency: "usd",
      stage: form.stage,
      probability: parseInt(form.probability) || 0,
      description: form.description,
    });
  }

  if (pipelineQuery.isLoading) {
    return (
      <div className="space-y-6">
        <Skeleton className="h-8 w-48" />
        <div className="grid gap-4 md:grid-cols-3 lg:grid-cols-6">
          {Array.from({ length: 6 }).map((_, i) => (
            <Skeleton key={i} className="h-64" />
          ))}
        </div>
      </div>
    );
  }

  const stages = pipelineQuery.data?.stages ?? [];

  return (
    <div className="space-y-6">
      <div className="flex items-center justify-between">
        <h1 className="text-2xl font-bold">{t("crm.pipeline")}</h1>
        <div className="flex items-center gap-2">
          <Badge variant="outline">
            {t("crm.totalValue")}: {formatAmount(pipelineQuery.data?.total_value ?? 0)}
          </Badge>
          <Button onClick={() => setCreateOpen(true)}>
            <Plus className="size-4" />
            {t("crm.newDeal")}
          </Button>
        </div>
      </div>

      <div className="grid gap-4 md:grid-cols-3 lg:grid-cols-6">
        {STAGE_ORDER.map((stageName) => {
          const stage = stages.find((s: PipelineStage) => s.stage === stageName);
          return (
            <Card key={stageName} className="flex flex-col">
              <CardHeader className="pb-2">
                <div className="flex items-center gap-2">
                  <div className={`size-2.5 rounded-full ${STAGE_COLORS[stageName] ?? "bg-gray-400"}`} />
                  <CardTitle className="text-xs font-medium uppercase tracking-wider">
                    {stageName.replace(/_/g, " ")}
                  </CardTitle>
                </div>
                <p className="text-[11px] text-muted-foreground">
                  {stage?.count ?? 0} {t("crm.deals").toLowerCase()} &middot; {formatAmount(stage?.total_amount ?? 0)}
                </p>
              </CardHeader>
              <CardContent className="flex-1 space-y-2 pt-0">
                {(stage?.deals ?? []).map((deal) => (
                  <div
                    key={deal.id}
                    className="rounded-lg border p-2.5 cursor-pointer hover:bg-muted/50 transition-colors"
                    onClick={() => router.push(`/admin/crm/deals/${deal.id}`)}
                  >
                    <p className="text-sm font-medium truncate">{deal.title}</p>
                    <div className="flex items-center justify-between mt-1">
                      <span className="text-xs text-muted-foreground">
                        {formatAmount(deal.amount ?? 0)}
                      </span>
                      <span className="text-xs text-muted-foreground">
                        {deal.probability}%
                      </span>
                    </div>
                    <div className="flex items-center gap-1 mt-1.5">
                      {stageName !== "closed_won" && stageName !== "closed_lost" && (
                        <Button
                          variant="ghost"
                          size="icon-sm"
                          onClick={(e) => {
                            e.stopPropagation();
                            handleAdvance(deal.id, deal.stage);
                          }}
                          title={t("crm.advanceStage")}
                        >
                          <ArrowRight className="size-3" />
                        </Button>
                      )}
                    </div>
                  </div>
                ))}
              </CardContent>
            </Card>
          );
        })}
      </div>

      <Dialog open={createOpen} onOpenChange={setCreateOpen}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>{t("crm.createDeal")}</DialogTitle>
          </DialogHeader>
          <div className="space-y-4">
            <div>
              <Label>{t("crm.dealTitle")}</Label>
              <Input
                value={form.title}
                onChange={(e) => setForm({ ...form, title: e.target.value })}
                placeholder={t("crm.dealTitlePlaceholder")}
              />
            </div>
            <div className="grid grid-cols-2 gap-4">
              <div>
                <Label>{t("crm.amount")}</Label>
                <Input
                  type="number"
                  value={form.amount}
                  onChange={(e) => setForm({ ...form, amount: e.target.value })}
                  placeholder="0.00"
                />
              </div>
              <div>
                <Label>{t("crm.probability")}</Label>
                <Input
                  type="number"
                  min={0}
                  max={100}
                  value={form.probability}
                  onChange={(e) => setForm({ ...form, probability: e.target.value })}
                />
              </div>
            </div>
            <div>
              <Label>{t("crm.stage")}</Label>
              <select
                className="flex h-9 w-full rounded-md border border-input bg-background px-3 py-1 text-sm"
                value={form.stage}
                onChange={(e) => setForm({ ...form, stage: e.target.value })}
              >
                {STAGE_ORDER.map((s) => (
                  <option key={s} value={s}>{s.replace(/_/g, " ")}</option>
                ))}
              </select>
            </div>
            <div>
              <Label>{t("common.description")}</Label>
              <Input
                value={form.description}
                onChange={(e) => setForm({ ...form, description: e.target.value })}
              />
            </div>
          </div>
          <DialogFooter>
            <Button variant="outline" onClick={() => setCreateOpen(false)}>
              {t("common.cancel")}
            </Button>
            <Button onClick={handleCreate} disabled={createMutation.isPending}>
              {createMutation.isPending ? t("common.creating") : t("common.create")}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </div>
  );
}
