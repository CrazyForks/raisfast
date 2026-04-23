"use client";

import { use, useState } from "react";
import Link from "next/link";
import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import { ArrowLeft, Plus } from "lucide-react";
import { toast } from "sonner";

import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Skeleton } from "@/components/ui/skeleton";
import { Badge } from "@/components/ui/badge";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Separator } from "@/components/ui/separator";
import { crm, type TimelineEvent } from "@/lib/crm";
import { useT } from "@/lib/i18n";

const LIFECYCLE_STAGES = [
  "subscriber", "lead", "marketing_qualified_lead",
  "sales_qualified_lead", "opportunity", "customer", "evangelist",
];

export default function ContactDetailPage({
  params,
}: {
  params: Promise<{ id: string }>;
}) {
  const { id } = use(params);
  const { t } = useT();
  const queryClient = useQueryClient();
  const [noteContent, setNoteContent] = useState("");

  const contactQuery = useQuery({
    queryKey: ["crm-contact", id],
    queryFn: () => crm.getContact(id),
  });

  const timelineQuery = useQuery({
    queryKey: ["crm-contact-timeline", id],
    queryFn: () => crm.getContactTimeline(id),
  });

  const convertMutation = useMutation({
    mutationFn: (stage: string) => crm.convertContactLifecycle(id, stage),
    onSuccess: () => {
      toast.success(t("crm.lifecycleUpdated"));
      queryClient.invalidateQueries({ queryKey: ["crm-contact", id] });
    },
    onError: () => toast.error(t("crm.failedToUpdateLifecycle")),
  });

  const addNoteMutation = useMutation({
    mutationFn: (content: string) => crm.createNote({ content, contact: id }),
    onSuccess: () => {
      setNoteContent("");
      toast.success(t("common.created", { name: t("crm.note") }));
      queryClient.invalidateQueries({ queryKey: ["crm-contact-timeline", id] });
    },
    onError: () => toast.error(t("common.failedToCreate", { name: t("crm.note") })),
  });

  if (contactQuery.isLoading) {
    return (
      <div className="space-y-6">
        <Skeleton className="h-8 w-64" />
        <Skeleton className="h-48" />
      </div>
    );
  }

  const contact = contactQuery.data;

  if (!contact) {
    return (
      <div className="space-y-6">
        <Link href="/admin/crm/contacts"><Button variant="outline" size="sm">{t("common.back")}</Button></Link>
        <p className="text-muted-foreground">{t("common.notFound")}</p>
      </div>
    );
  }

  return (
    <div className="space-y-6">
      <div className="flex items-center gap-4">
        <Link href="/admin/crm/contacts">
          <Button variant="outline" size="sm"><ArrowLeft className="size-4" /></Button>
        </Link>
        <h1 className="text-2xl font-bold">
          {contact.first_name} {contact.last_name}
        </h1>
      </div>

      <div className="grid gap-6 md:grid-cols-3">
        <div className="md:col-span-2 space-y-6">
          <Card>
            <CardHeader>
              <CardTitle>{t("crm.contactInfo")}</CardTitle>
            </CardHeader>
            <CardContent className="space-y-3">
              <div className="grid grid-cols-2 gap-4 text-sm">
                <div><span className="text-muted-foreground">{t("crm.email")}:</span> {contact.email ?? "-"}</div>
                <div><span className="text-muted-foreground">{t("crm.phone")}:</span> {contact.phone ?? "-"}</div>
                <div><span className="text-muted-foreground">{t("crm.jobTitle")}:</span> {contact.job_title ?? "-"}</div>
                <div><span className="text-muted-foreground">{t("crm.company")}:</span> {contact.company ?? "-"}</div>
                <div><span className="text-muted-foreground">{t("crm.source")}:</span> {contact.source ? <Badge variant="secondary">{contact.source.replace(/_/g, " ")}</Badge> : "-"}</div>
                <div><span className="text-muted-foreground">{t("crm.status")}:</span> {contact.status ? <Badge variant="outline">{contact.status}</Badge> : "-"}</div>
              </div>
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

          <Card>
            <CardHeader>
              <CardTitle>{t("crm.addNote")}</CardTitle>
            </CardHeader>
            <CardContent>
              <div className="flex gap-2">
                <Input
                  value={noteContent}
                  onChange={(e) => setNoteContent(e.target.value)}
                  placeholder={t("crm.notePlaceholder")}
                  onKeyDown={(e) => {
                    if (e.key === "Enter" && noteContent.trim()) {
                      addNoteMutation.mutate(noteContent);
                    }
                  }}
                />
                <Button
                  onClick={() => addNoteMutation.mutate(noteContent)}
                  disabled={!noteContent.trim() || addNoteMutation.isPending}
                >
                  <Plus className="size-4" />
                </Button>
              </div>
            </CardContent>
          </Card>
        </div>

        <div className="space-y-6">
          <Card>
            <CardHeader>
              <CardTitle>{t("crm.lifecycleStage")}</CardTitle>
            </CardHeader>
            <CardContent className="space-y-2">
              <Badge variant="outline" className="text-sm">
                {(contact.lifecycle_stage ?? "lead").replace(/_/g, " ")}
              </Badge>
              <select
                className="flex h-9 w-full rounded-md border border-input bg-background px-3 py-1 text-sm"
                value={contact.lifecycle_stage ?? "lead"}
                onChange={(e) => convertMutation.mutate(e.target.value)}
              >
                {LIFECYCLE_STAGES.map((s) => (
                  <option key={s} value={s}>{s.replace(/_/g, " ")}</option>
                ))}
              </select>
            </CardContent>
          </Card>

          {contact.notes && (
            <Card>
              <CardHeader>
                <CardTitle>{t("crm.notes")}</CardTitle>
              </CardHeader>
              <CardContent>
                <p className="text-sm text-muted-foreground">{contact.notes}</p>
              </CardContent>
            </Card>
          )}
        </div>
      </div>
    </div>
  );
}
