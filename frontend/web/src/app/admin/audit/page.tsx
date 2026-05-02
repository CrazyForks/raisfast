"use client";

import { useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { ClipboardList, Search, Download } from "lucide-react";

import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Badge } from "@/components/ui/badge";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { client } from "@/lib/raisfast";
import { useT } from "@/lib/i18n";

interface AuditEntry {
  id: string;
  tenant_id: string;
  actor_id: string | null;
  actor_role: string | null;
  action: string;
  subject: string;
  subject_id: string | null;
  detail: string | null;
  ip_address: string | null;
  user_agent: string | null;
  created_at: string;
}

interface PaginatedData<T> {
  items: T[];
  total: number;
  page: number;
  page_size: number;
}

const ACTION_COLORS: Record<
  string,
  "default" | "secondary" | "destructive" | "outline"
> = {
  create: "default",
  update: "secondary",
  delete: "destructive",
  upload: "default",
  register: "outline",
  login: "secondary",
};

function todayStr(): string {
  return new Date().toISOString().split("T")[0];
}

function daysAgo(n: number): string {
  const d = new Date();
  d.setDate(d.getDate() - n);
  return d.toISOString().split("T")[0];
}

export default function AuditPage() {
  const { t } = useT();
  const [page, setPage] = useState(1);
  const [actionFilter, setActionFilter] = useState("");
  const [searchAction, setSearchAction] = useState("");
  const [dateFrom, setDateFrom] = useState("");
  const [dateTo, setDateTo] = useState("");
  const [appliedFrom, setAppliedFrom] = useState("");
  const [appliedTo, setAppliedTo] = useState("");
  const pageSize = 20;

  const queryParams = new URLSearchParams({
    page: String(page),
    page_size: String(pageSize),
  });
  if (searchAction) {
    queryParams.set("action", searchAction);
  }

  const auditQuery = useQuery({
    queryKey: ["audit", page, searchAction],
    queryFn: () =>
      client.send<PaginatedData<AuditEntry>>(
        `/admin/audit`,
        { query: Object.fromEntries(queryParams.entries()) },
      ),
  });

  function handleSearch() {
    setSearchAction(actionFilter);
    setAppliedFrom(dateFrom);
    setAppliedTo(dateTo);
    setPage(1);
  }

  function clearFilter() {
    setActionFilter("");
    setSearchAction("");
    setDateFrom("");
    setDateTo("");
    setAppliedFrom("");
    setAppliedTo("");
    setPage(1);
  }

  function applyDateRange(range: string) {
    switch (range) {
      case "today":
        setDateFrom(todayStr());
        setDateTo(todayStr());
        break;
      case "7d":
        setDateFrom(daysAgo(7));
        setDateTo(todayStr());
        break;
      case "30d":
        setDateFrom(daysAgo(30));
        setDateTo(todayStr());
        break;
      default:
        setDateFrom("");
        setDateTo("");
    }
  }

  function filterByDate(entries: AuditEntry[]): AuditEntry[] {
    let filtered = entries;
    if (appliedFrom) {
      const from = new Date(appliedFrom);
      filtered = filtered.filter(
        (e) => new Date(e.created_at) >= from,
      );
    }
    if (appliedTo) {
      const to = new Date(appliedTo);
      to.setHours(23, 59, 59, 999);
      filtered = filtered.filter(
        (e) => new Date(e.created_at) <= to,
      );
    }
    return filtered;
  }

  function exportCsv() {
    const entries = filterByDate(auditQuery.data?.items ?? []);
    const header =
      "Time,Action,Subject,Subject ID,Actor ID,Actor Role,Detail,IP Address";
    const rows = entries.map((e) =>
      [
        e.created_at,
        e.action,
        e.subject,
        e.subject_id ?? "",
        e.actor_id ?? "system",
        e.actor_role ?? "",
        (e.detail ?? "").replace(/"/g, '""'),
        e.ip_address ?? "",
      ]
        .map((v) => `"${v}"`)
        .join(","),
    );
    const csv = [header, ...rows].join("\n");
    const blob = new Blob([csv], { type: "text/csv;charset=utf-8;" });
    const url = URL.createObjectURL(blob);
    const a = document.createElement("a");
    a.href = url;
    a.download = `audit-log-${todayStr()}.csv`;
    a.click();
    URL.revokeObjectURL(url);
  }

  function exportJson() {
    const entries = filterByDate(auditQuery.data?.items ?? []);
    const blob = new Blob([JSON.stringify(entries, null, 2)], {
      type: "application/json",
    });
    const url = URL.createObjectURL(blob);
    const a = document.createElement("a");
    a.href = url;
    a.download = `audit-log-${todayStr()}.json`;
    a.click();
    URL.revokeObjectURL(url);
  }

  const entries = filterByDate(auditQuery.data?.items ?? []);
  const totalPages = Math.ceil((auditQuery.data?.total ?? 0) / pageSize);

  return (
    <div className="space-y-6">
      <div className="flex items-center justify-between">
        <div className="flex items-center gap-2">
          <ClipboardList className="size-6" />
          <h1 className="text-2xl font-bold">{t("audit.title")}</h1>
        </div>
        <div className="flex items-center gap-2">
          <Button variant="outline" size="sm" onClick={exportCsv}>
            <Download className="size-4" />
            CSV
          </Button>
          <Button variant="outline" size="sm" onClick={exportJson}>
            <Download className="size-4" />
            JSON
          </Button>
        </div>
      </div>

      <div className="flex items-end gap-3 flex-wrap">
        <div className="space-y-1">
          <Label className="text-xs">{t("audit.action")}</Label>
          <Input
            placeholder="create, delete, ..."
            value={actionFilter}
            onChange={(e) => setActionFilter(e.target.value)}
            onKeyDown={(e) => e.key === "Enter" && handleSearch()}
            className="w-48"
          />
        </div>
        <div className="space-y-1">
          <Label className="text-xs">{t("audit.from")}</Label>
          <Input
            type="date"
            value={dateFrom}
            onChange={(e) => setDateFrom(e.target.value)}
            className="w-40"
          />
        </div>
        <div className="space-y-1">
          <Label className="text-xs">{t("audit.to")}</Label>
          <Input
            type="date"
            value={dateTo}
            onChange={(e) => setDateTo(e.target.value)}
            className="w-40"
          />
        </div>
        <Button variant="outline" size="sm" onClick={handleSearch}>
          <Search className="size-4" />
          {t("audit.search")}
        </Button>
        <Button variant="ghost" size="sm" onClick={clearFilter}>
          {t("audit.clear")}
        </Button>
        <div className="flex items-center gap-1 ml-auto">
          <Button variant="ghost" size="sm" onClick={() => applyDateRange("today")}>
            {t("audit.today")}
          </Button>
          <Button variant="ghost" size="sm" onClick={() => applyDateRange("7d")}>
            7d
          </Button>
          <Button variant="ghost" size="sm" onClick={() => applyDateRange("30d")}>
            30d
          </Button>
        </div>
      </div>

      {auditQuery.error && (
        <div className="text-sm text-destructive bg-destructive/10 p-3 rounded-md">
          {t("audit.failedToLoad")}
        </div>
      )}

      <Card>
        <CardContent className="p-0">
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead>{t("audit.timestamp")}</TableHead>
                <TableHead>{t("audit.action")}</TableHead>
                <TableHead>{t("audit.resource")}</TableHead>
                <TableHead>{t("audit.subjectId")}</TableHead>
                <TableHead>{t("audit.actor")}</TableHead>
                <TableHead>{t("audit.detailCol")}</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {auditQuery.isLoading ? (
                <TableRow>
                  <TableCell colSpan={6} className="text-center py-8">
                    {t("common.loading")}
                  </TableCell>
                </TableRow>
              ) : entries.length === 0 ? (
                <TableRow>
                  <TableCell colSpan={6} className="text-center py-8">
                    {t("audit.noEntries")}
                  </TableCell>
                </TableRow>
              ) : (
                entries.map((entry) => (
                  <TableRow key={entry.id}>
                    <TableCell className="text-xs text-muted-foreground whitespace-nowrap">
                      {new Date(entry.created_at).toLocaleString()}
                    </TableCell>
                    <TableCell>
                      <Badge variant={ACTION_COLORS[entry.action] ?? "outline"}>
                        {entry.action}
                      </Badge>
                    </TableCell>
                    <TableCell className="font-medium">
                      {entry.subject}
                    </TableCell>
                    <TableCell className="font-mono text-xs max-w-32 truncate">
                      {entry.subject_id
                        ? entry.subject_id.slice(0, 8) + "..."
                        : "—"}
                    </TableCell>
                    <TableCell className="text-xs">
                      {entry.actor_id ? (
                        <span className="font-mono">
                          {entry.actor_id.slice(0, 8)}
                        </span>
                      ) : (
                        <span className="text-muted-foreground">{t("audit.system")}</span>
                      )}
                      {entry.actor_role && (
                        <Badge
                          variant="outline"
                          className="ml-1 text-[10px] px-1"
                        >
                          {entry.actor_role}
                        </Badge>
                      )}
                    </TableCell>
                    <TableCell className="text-xs text-muted-foreground max-w-48 truncate">
                      {entry.detail || "—"}
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
          <Button
            variant="outline"
            size="sm"
            disabled={page <= 1}
            onClick={() => setPage((p) => p - 1)}
          >
            {t("common.previous")}
          </Button>
          <span className="text-sm text-muted-foreground">
            Page {t("common.pageOf", { page, total: totalPages })}
          </span>
          <Button
            variant="outline"
            size="sm"
            disabled={page >= totalPages}
            onClick={() => setPage((p) => p + 1)}
          >
            {t("common.next")}
          </Button>
        </div>
      )}
    </div>
  );
}
