"use client";

import { useQuery } from "@tanstack/react-query";
import { BarChart3, Activity } from "lucide-react";

import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Skeleton } from "@/components/ui/skeleton";
import { Badge } from "@/components/ui/badge";
import { crm } from "@/lib/crm";
import { useT } from "@/lib/i18n";

export default function ReportsPage() {
  const { t } = useT();

  const funnelQuery = useQuery({
    queryKey: ["crm-funnel-report"],
    queryFn: crm.getFunnelReport,
  });

  const activityQuery = useQuery({
    queryKey: ["crm-activity-report"],
    queryFn: crm.getActivityReport,
  });

  return (
    <div className="space-y-6">
      <h1 className="text-2xl font-bold">{t("crm.reports")}</h1>

      <div className="grid gap-6 md:grid-cols-2">
        <Card>
          <CardHeader>
            <CardTitle className="flex items-center gap-2">
              <BarChart3 className="size-4" />
              {t("crm.funnelReport")}
            </CardTitle>
          </CardHeader>
          <CardContent>
            {funnelQuery.isLoading ? (
              <Skeleton className="h-48" />
            ) : !funnelQuery.data?.stages?.length ? (
              <p className="text-sm text-muted-foreground">{t("crm.noData")}</p>
            ) : (
              <div className="space-y-3">
                {funnelQuery.data.stages.map((stage) => (
                  <div key={stage.stage} className="space-y-1">
                    <div className="flex items-center justify-between text-sm">
                      <span className="font-medium">{stage.stage.replace(/_/g, " ")}</span>
                      <span className="text-muted-foreground">
                        {stage.count} {t("crm.deals").toLowerCase()} &middot; ${(stage.value / 100).toLocaleString()}
                      </span>
                    </div>
                    <div className="h-2 rounded-full bg-muted overflow-hidden">
                      <div
                        className="h-full bg-primary rounded-full"
                        style={{
                          width: `${Math.max(stage.conversion_rate * 100, 2)}%`,
                        }}
                      />
                    </div>
                    <p className="text-[11px] text-muted-foreground">
                      {t("crm.conversionRate")}: {(stage.conversion_rate * 100).toFixed(1)}%
                    </p>
                  </div>
                ))}
              </div>
            )}
          </CardContent>
        </Card>

        <Card>
          <CardHeader>
            <CardTitle className="flex items-center gap-2">
              <Activity className="size-4" />
              {t("crm.activityReport")}
            </CardTitle>
          </CardHeader>
          <CardContent>
            {activityQuery.isLoading ? (
              <Skeleton className="h-48" />
            ) : !activityQuery.data ? (
              <p className="text-sm text-muted-foreground">{t("crm.noData")}</p>
            ) : (
              <div className="space-y-4">
                <div>
                  <p className="text-sm font-medium mb-2">{t("crm.byType")}</p>
                  <div className="flex flex-wrap gap-2">
                    {Object.entries(activityQuery.data.by_type ?? {}).map(([type, count]) => (
                      <Badge key={type} variant="secondary">
                        {type.replace(/_/g, " ")}: {count as number}
                      </Badge>
                    ))}
                  </div>
                </div>
                <div>
                  <p className="text-sm font-medium mb-2">{t("crm.byOutcome")}</p>
                  <div className="flex flex-wrap gap-2">
                    {Object.entries(activityQuery.data.by_outcome ?? {}).map(([outcome, count]) => (
                      <Badge key={outcome} variant="outline">
                        {outcome.replace(/_/g, " ")}: {count as number}
                      </Badge>
                    ))}
                  </div>
                </div>
                <div>
                  <p className="text-sm font-medium">
                    {t("crm.totalActivities")}: {activityQuery.data.total ?? 0}
                  </p>
                </div>
              </div>
            )}
          </CardContent>
        </Card>
      </div>
    </div>
  );
}
