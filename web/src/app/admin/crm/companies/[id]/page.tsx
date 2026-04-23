"use client";

import { use } from "react";
import Link from "next/link";
import { useQuery } from "@tanstack/react-query";
import { ArrowLeft } from "lucide-react";

import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Skeleton } from "@/components/ui/skeleton";
import { Badge } from "@/components/ui/badge";
import { crm, type TimelineEvent } from "@/lib/crm";
import { useT } from "@/lib/i18n";

export default function CompanyDetailPage({
  params,
}: {
  params: Promise<{ id: string }>;
}) {
  const { id } = use(params);
  const { t } = useT();

  const companyQuery = useQuery({
    queryKey: ["crm-company", id],
    queryFn: () => crm.getCompany(id),
  });

  const timelineQuery = useQuery({
    queryKey: ["crm-company-timeline", id],
    queryFn: () => crm.getCompanyTimeline(id),
  });

  if (companyQuery.isLoading) {
    return (
      <div className="space-y-6">
        <Skeleton className="h-8 w-64" />
        <Skeleton className="h-48" />
      </div>
    );
  }

  const company = companyQuery.data;

  if (!company) {
    return (
      <div className="space-y-6">
        <Link href="/admin/crm/companies"><Button variant="outline" size="sm">{t("common.back")}</Button></Link>
        <p className="text-muted-foreground">{t("common.notFound")}</p>
      </div>
    );
  }

  return (
    <div className="space-y-6">
      <div className="flex items-center gap-4">
        <Link href="/admin/crm/companies">
          <Button variant="outline" size="sm"><ArrowLeft className="size-4" /></Button>
        </Link>
        <h1 className="text-2xl font-bold">{company.name}</h1>
      </div>

      <div className="grid gap-6 md:grid-cols-3">
        <div className="md:col-span-2 space-y-6">
          <Card>
            <CardHeader>
              <CardTitle>{t("crm.companyInfo")}</CardTitle>
            </CardHeader>
            <CardContent className="space-y-3">
              <div className="grid grid-cols-2 gap-4 text-sm">
                <div><span className="text-muted-foreground">{t("crm.website")}:</span> {company.website ?? "-"}</div>
                <div><span className="text-muted-foreground">{t("crm.phone")}:</span> {company.phone ?? "-"}</div>
                <div><span className="text-muted-foreground">{t("crm.industry")}:</span> {company.industry ? <Badge variant="secondary">{company.industry}</Badge> : "-"}</div>
                <div><span className="text-muted-foreground">{t("crm.size")}:</span> {company.size ?? "-"}</div>
                <div><span className="text-muted-foreground">{t("crm.city")}:</span> {company.city ?? "-"}</div>
                <div><span className="text-muted-foreground">{t("crm.country")}:</span> {company.country ?? "-"}</div>
              </div>
              {company.description && (
                <div className="text-sm"><span className="text-muted-foreground">{t("common.description")}:</span> {company.description}</div>
              )}
            </CardContent>
          </Card>

          <Card>
            <CardHeader>
              <CardTitle>{t("crm.timeline")}</CardTitle>
            </CardHeader>
            <CardContent>
              {timelineQuery.isLoading ? (
                <Skeleton className="h-32" />
              ) : !timelineQuery.data?.length ? (
                <p className="text-sm text-muted-foreground">{t("crm.noTimeline")}</p>
              ) : (
                <div className="space-y-3">
                  {timelineQuery.data.map((event: TimelineEvent) => (
                    <div key={event.id} className="flex gap-3 text-sm">
                      <div className="size-2 rounded-full bg-primary mt-1.5 shrink-0" />
                      <div className="flex-1">
                        <p className="font-medium">{event.subject ?? event.type}</p>
                        {event.content && <p className="text-muted-foreground">{event.content}</p>}
                        <p className="text-[11px] text-muted-foreground">{new Date(event.created_at).toLocaleString()}</p>
                      </div>
                    </div>
                  ))}
                </div>
              )}
            </CardContent>
          </Card>
        </div>
      </div>
    </div>
  );
}
