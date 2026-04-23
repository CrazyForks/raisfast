"use client";

import { useQuery } from "@tanstack/react-query";
import {
  Building2,
  Users,
  DollarSign,
  TrendingUp,
  Activity,
  BarChart3,
} from "lucide-react";

import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Skeleton } from "@/components/ui/skeleton";
import { Badge } from "@/components/ui/badge";
import { crm } from "@/lib/crm";
import { useT } from "@/lib/i18n";

function StatCard({
  title,
  value,
  subtitle,
  icon: Icon,
}: {
  title: string;
  value: string | number;
  subtitle?: string;
  icon: React.ElementType;
}) {
  return (
    <Card>
      <CardHeader className="flex flex-row items-center justify-between pb-2">
        <CardTitle className="text-sm font-medium">{title}</CardTitle>
        <Icon className="size-4 text-muted-foreground" />
      </CardHeader>
      <CardContent>
        <div className="text-2xl font-bold">{value}</div>
        {subtitle && (
          <p className="text-xs text-muted-foreground">{subtitle}</p>
        )}
      </CardContent>
    </Card>
  );
}

export default function CrmDashboardPage() {
  const { t } = useT();

  const statsQuery = useQuery({
    queryKey: ["crm-stats"],
    queryFn: crm.getStats,
  });

  const leaderboardQuery = useQuery({
    queryKey: ["crm-leaderboard"],
    queryFn: crm.getLeaderboard,
  });

  if (statsQuery.isLoading) {
    return (
      <div className="space-y-6">
        <Skeleton className="h-8 w-48" />
        <div className="grid gap-4 md:grid-cols-2 lg:grid-cols-4">
          {Array.from({ length: 4 }).map((_, i) => (
            <Skeleton key={i} className="h-28" />
          ))}
        </div>
      </div>
    );
  }

  const stats = statsQuery.data;

  return (
    <div className="space-y-6">
      <h1 className="text-2xl font-bold">{t("crm.dashboard")}</h1>

      <div className="grid gap-4 md:grid-cols-2 lg:grid-cols-4">
        <StatCard
          title={t("crm.totalCompanies")}
          value={stats?.total_companies ?? 0}
          icon={Building2}
        />
        <StatCard
          title={t("crm.totalContacts")}
          value={stats?.total_contacts ?? 0}
          icon={Users}
        />
        <StatCard
          title={t("crm.openDeals")}
          value={stats?.open_deals ?? 0}
          subtitle={`${t("crm.wonDeals")}: ${stats?.won_deals ?? 0}`}
          icon={TrendingUp}
        />
        <StatCard
          title={t("crm.pipelineValue")}
          value={`$${((stats?.total_pipeline_value ?? 0) / 100).toLocaleString()}`}
          subtitle={`${t("crm.weightedValue")}: $${((stats?.weighted_pipeline_value ?? 0) / 100).toLocaleString()}`}
          icon={DollarSign}
        />
      </div>

      <div className="grid gap-4 md:grid-cols-2 lg:grid-cols-3">
        <Card>
          <CardHeader>
            <CardTitle className="flex items-center gap-2">
              <BarChart3 className="size-4" />
              {t("crm.winRate")}
            </CardTitle>
          </CardHeader>
          <CardContent>
            <div className="text-3xl font-bold">
              {((stats?.win_rate ?? 0) * 100).toFixed(1)}%
            </div>
            <p className="text-sm text-muted-foreground">
              {t("crm.avgDealSize")}: ${((stats?.avg_deal_size ?? 0) / 100).toLocaleString()}
            </p>
          </CardContent>
        </Card>

        <Card>
          <CardHeader>
            <CardTitle className="flex items-center gap-2">
              <Activity className="size-4" />
              {t("crm.recentActivities")}
            </CardTitle>
          </CardHeader>
          <CardContent>
            <div className="text-3xl font-bold">
              {stats?.activities_this_week ?? 0}
            </div>
            <p className="text-sm text-muted-foreground">
              {t("crm.totalActivities")}: {stats?.total_activities ?? 0}
            </p>
          </CardContent>
        </Card>

        <Card>
          <CardHeader>
            <CardTitle className="flex items-center gap-2">
              <TrendingUp className="size-4" />
              {t("crm.leaderboard")}
            </CardTitle>
          </CardHeader>
          <CardContent>
            {leaderboardQuery.isLoading ? (
              <Skeleton className="h-24" />
            ) : !leaderboardQuery.data?.length ? (
              <p className="text-sm text-muted-foreground">{t("crm.noData")}</p>
            ) : (
              <div className="space-y-2">
                {leaderboardQuery.data.slice(0, 5).map((entry, i) => (
                  <div key={entry.owner_id} className="flex items-center justify-between text-sm">
                    <div className="flex items-center gap-2">
                      <Badge variant="secondary" className="size-5 p-0 text-[10px] flex items-center justify-center">
                        {i + 1}
                      </Badge>
                      <span>{entry.owner_name ?? entry.owner_id.slice(0, 8)}</span>
                    </div>
                    <span className="font-medium">${((entry.won_value ?? 0) / 100).toLocaleString()}</span>
                  </div>
                ))}
              </div>
            )}
          </CardContent>
        </Card>
      </div>
    </div>
  );
}
