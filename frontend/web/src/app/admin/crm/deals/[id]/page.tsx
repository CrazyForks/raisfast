"use client";

import { use } from "react";
import Link from "next/link";
import { useQuery } from "@tanstack/react-query";
import { ArrowLeft, Plus } from "lucide-react";
import { toast } from "sonner";

import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Skeleton } from "@/components/ui/skeleton";
import { Badge } from "@/components/ui/badge";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Separator } from "@/components/ui/separator";
import { crm, type Activity, type Note } from "@/lib/crm";
import { useT } from "@/lib/i18n";

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

export default function DealDetailPage({
  params,
}: {
  params: Promise<{ id: string }>;
}) {
  const { id } = use(params);
  const { t } = useT();

  const dealQuery = useQuery({
    queryKey: ["crm-deal", id],
    queryFn: () => crm.getDeal(id),
  });

  const detailQuery = useQuery({
    queryKey: ["crm-deal-detail", id],
    queryFn: () => crm.getDealDetail(id),
  });

  if (dealQuery.isLoading) {
    return (
      <div className="space-y-6">
        <Skeleton className="h-8 w-64" />
        <Skeleton className="h-48" />
      </div>
    );
  }

  const deal = dealQuery.data;
  if (!deal) {
    return (
      <div className="space-y-6">
        <Link href="/admin/crm/deals"><Button variant="outline" size="sm">{t("common.back")}</Button></Link>
        <p className="text-muted-foreground">{t("common.notFound")}</p>
      </div>
    );
  }

  const activities = detailQuery.data?.activities ?? [];
  const notes = detailQuery.data?.notes ?? [];

  return (
    <div className="space-y-6">
      <div className="flex items-center gap-4">
        <Link href="/admin/crm/deals">
          <Button variant="outline" size="sm"><ArrowLeft className="size-4" /></Button>
        </Link>
        <h1 className="text-2xl font-bold">{deal.title}</h1>
        <Badge variant="outline" className="flex items-center gap-1.5">
          <div className={`size-2 rounded-full ${STAGE_COLORS[deal.stage] ?? "bg-gray-400"}`} />
          {(deal.stage ?? "").replace(/_/g, " ")}
        </Badge>
      </div>

      <div className="grid gap-6 md:grid-cols-3">
        <div className="md:col-span-2 space-y-6">
          <Card>
            <CardHeader>
              <CardTitle>{t("crm.activities")}</CardTitle>
            </CardHeader>
            <CardContent>
              {!activities.length ? (
                <p className="text-sm text-muted-foreground">{t("crm.noActivities")}</p>
              ) : (
                <div className="space-y-3">
                  {activities.map((activity: Activity) => (
                    <div key={activity.id} className="flex gap-3 text-sm">
                      <div className="size-2 rounded-full bg-primary mt-1.5 shrink-0" />
                      <div className="flex-1">
                        <p className="font-medium">{activity.subject ?? activity.type}</p>
                        {activity.content && <p className="text-muted-foreground">{activity.content}</p>}
                        <p className="text-[11px] text-muted-foreground">
                          {activity.activity_date ?? activity.created_at}
                          {activity.duration_minutes != null && ` · ${activity.duration_minutes}min`}
                          {activity.outcome && ` · ${activity.outcome}`}
                        </p>
                      </div>
                    </div>
                  ))}
                </div>
              )}
            </CardContent>
          </Card>

          <Card>
            <CardHeader>
              <CardTitle>{t("crm.notes")}</CardTitle>
            </CardHeader>
            <CardContent>
              {!notes.length ? (
                <p className="text-sm text-muted-foreground">{t("crm.noNotes")}</p>
              ) : (
                <div className="space-y-3">
                  {notes.map((note: Note) => (
                    <div key={note.id} className="rounded-lg border p-3 text-sm">
                      <p>{note.content}</p>
                      <p className="text-[11px] text-muted-foreground mt-1">{new Date(note.created_at).toLocaleString()}</p>
                    </div>
                  ))}
                </div>
              )}
            </CardContent>
          </Card>
        </div>

        <Card>
          <CardHeader>
            <CardTitle>{t("crm.dealInfo")}</CardTitle>
          </CardHeader>
          <CardContent className="space-y-3 text-sm">
            <div><span className="text-muted-foreground">{t("crm.amount")}:</span> {formatAmount(deal.amount)}</div>
            <div><span className="text-muted-foreground">{t("crm.probability")}:</span> {deal.probability ?? 0}%</div>
            <div><span className="text-muted-foreground">{t("crm.closeDate")}:</span> {deal.close_date ?? "-"}</div>
            {deal.description && <div><span className="text-muted-foreground">{t("common.description")}:</span> {deal.description}</div>}
            {deal.loss_reason && <div><span className="text-muted-foreground">{t("crm.lossReason")}:</span> {deal.loss_reason.replace(/_/g, " ")}</div>}
          </CardContent>
        </Card>
      </div>
    </div>
  );
}
