"use client";

import { useEffect, useState } from "react";
import Link from "next/link";
import {
  FileText,
  MessageSquare,
  Image,
  Users,
  Folder,
  Tag,
  TrendingUp,
} from "lucide-react";
import { useQuery } from "@tanstack/react-query";

import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Skeleton } from "@/components/ui/skeleton";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { api } from "@/lib/api";
import { useAuthStore } from "@/stores/auth";

interface StatsOverview {
  total_posts: number;
  total_comments: number;
  total_users: number;
  total_media: number;
  total_categories: number;
  total_tags: number;
  posts_by_status: Record<string, number>;
  comments_by_status: Record<string, number>;
  content_by_type: Record<string, number>;
  recent_activity: RecentActivity[];
}

interface RecentActivity {
  type: string;
  title?: string;
  slug?: string;
  content?: string;
  at: string;
}

interface TrendsData {
  table: string;
  days: number;
  data: { date: string; count: number }[];
}

const POST_STATUS_COLORS: Record<string, "default" | "secondary" | "outline" | "destructive"> = {
  published: "default",
  draft: "secondary",
  archived: "outline",
};

const COMMENT_STATUS_COLORS: Record<string, "default" | "secondary" | "outline" | "destructive"> = {
  approved: "default",
  pending: "secondary",
  rejected: "destructive",
};

function StatusBadges({
  label,
  icon: Icon,
  total,
  byStatus,
  colorMap,
  href,
}: {
  label: string;
  icon: React.ComponentType<{ className?: string }>;
  total: number;
  byStatus: Record<string, number>;
  colorMap: Record<string, "default" | "secondary" | "outline" | "destructive">;
  href: string;
}) {
  return (
    <Link href={href}>
      <Card className="hover:bg-muted/50 transition-colors cursor-pointer">
        <CardHeader className="flex flex-row items-center justify-between pb-2">
          <CardTitle className="text-sm font-medium">{label}</CardTitle>
          <Icon className="size-4 text-muted-foreground" />
        </CardHeader>
        <CardContent>
          <div className="text-2xl font-bold">{total}</div>
          <div className="flex flex-wrap gap-1 mt-2">
            {Object.entries(byStatus).map(([status, count]) => (
              <Badge key={status} variant={colorMap[status] ?? "outline"} className="text-xs">
                {status}: {count}
              </Badge>
            ))}
          </div>
        </CardContent>
      </Card>
    </Link>
  );
}

export default function DashboardPage() {
  const { isAdmin } = useAuthStore();
  const [mounted, setMounted] = useState(false);

  useEffect(() => {
    setMounted(true);
  }, []);

  const statsQuery = useQuery({
    queryKey: ["admin-stats"],
    queryFn: () => api.get<StatsOverview>("/admin/stats"),
    refetchInterval: 30000,
  });

  const trendsQuery = useQuery({
    queryKey: ["admin-stats-trends", "posts", 14],
    queryFn: () =>
      api.get<TrendsData>("/admin/stats/trends?table=posts&days=14"),
    refetchInterval: 60000,
  });

  const overview = statsQuery.data;
  const recentActivity = overview?.recent_activity ?? [];
  const trendsData = trendsQuery.data?.data ?? [];

  const maxTrend = Math.max(...trendsData.map((d) => d.count), 1);

  const simpleCards = [
    {
      label: "Categories",
      value: overview?.total_categories,
      icon: Folder,
      href: "/admin/categories",
    },
    {
      label: "Tags",
      value: overview?.total_tags,
      icon: Tag,
      href: "/admin/tags",
    },
    {
      label: "Media",
      value: overview?.total_media,
      icon: Image,
      href: "/admin/media",
    },
    ...(isAdmin()
      ? [
          {
            label: "Users",
            value: overview?.total_users,
            icon: Users,
            href: "/admin/users",
          },
        ]
      : []),
  ];

  if (!mounted) {
    return (
      <div className="space-y-6">
        <h1 className="text-2xl font-bold">Dashboard</h1>
        <div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-3 xl:grid-cols-5">
          {Array.from({ length: 5 }).map((_, i) => (
            <Card key={i}>
              <CardHeader className="pb-2">
                <Skeleton className="h-4 w-20" />
              </CardHeader>
              <CardContent>
                <Skeleton className="h-8 w-16" />
              </CardContent>
            </Card>
          ))}
        </div>
      </div>
    );
  }

  return (
    <div className="space-y-6">
      <h1 className="text-2xl font-bold">Dashboard</h1>

      {statsQuery.error && (
        <div className="text-sm text-destructive bg-destructive/10 p-3 rounded-md flex items-center justify-between">
          <span>Failed to load dashboard stats. Data may be stale.</span>
          <Button
            variant="ghost"
            size="sm"
            onClick={() => statsQuery.refetch()}
          >
            Retry
          </Button>
        </div>
      )}

      <div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-3 xl:grid-cols-5">
        <StatusBadges
          label="Posts"
          icon={FileText}
          total={overview?.total_posts ?? 0}
          byStatus={overview?.posts_by_status ?? {}}
          colorMap={POST_STATUS_COLORS}
          href="/admin/posts"
        />
        <StatusBadges
          label="Comments"
          icon={MessageSquare}
          total={overview?.total_comments ?? 0}
          byStatus={overview?.comments_by_status ?? {}}
          colorMap={COMMENT_STATUS_COLORS}
          href="/admin/comments"
        />
        {simpleCards.map((stat) => (
          <Link key={stat.label} href={stat.href}>
            <Card className="hover:bg-muted/50 transition-colors cursor-pointer">
              <CardHeader className="flex flex-row items-center justify-between pb-2">
                <CardTitle className="text-sm font-medium">
                  {stat.label}
                </CardTitle>
                <stat.icon className="size-4 text-muted-foreground" />
              </CardHeader>
              <CardContent>
                {stat.value === undefined ? (
                  <Skeleton className="h-8 w-16" />
                ) : (
                  <div className="text-2xl font-bold">{stat.value}</div>
                )}
              </CardContent>
            </Card>
          </Link>
        ))}
      </div>

      <div className="grid gap-4 lg:grid-cols-2">
        <Card>
          <CardHeader>
            <div className="flex items-center justify-between">
              <CardTitle className="flex items-center gap-2">
                <TrendingUp className="size-4" />
                Posts (Last 14 Days)
              </CardTitle>
              <Link href="/admin/posts">
                <Button variant="outline" size="sm">
                  View All
                </Button>
              </Link>
            </div>
          </CardHeader>
          <CardContent>
            {trendsQuery.error ? (
              <div className="flex items-center justify-center h-40 text-sm text-muted-foreground">
                Failed to load trends.
                <Button
                  variant="ghost"
                  size="sm"
                  className="ml-2"
                  onClick={() => trendsQuery.refetch()}
                >
                  Retry
                </Button>
              </div>
            ) : trendsQuery.isLoading ? (
              <Skeleton className="h-40 w-full" />
            ) : trendsData.length === 0 ? (
              <p className="text-sm text-muted-foreground">No data yet.</p>
            ) : (
              <div className="flex items-end gap-1 h-40">
                {trendsData.map((d) => (
                  <div
                    key={d.date}
                    className="flex-1 bg-primary/80 rounded-t hover:bg-primary transition-colors relative group"
                    style={{
                      height: `${(d.count / maxTrend) * 100}%`,
                      minHeight: d.count > 0 ? "4px" : "0px",
                    }}
                    title={`${d.date}: ${d.count}`}
                  >
                    <div className="absolute -top-6 left-1/2 -translate-x-1/2 text-xs opacity-0 group-hover:opacity-100 transition-opacity whitespace-nowrap">
                      {d.count}
                    </div>
                  </div>
                ))}
              </div>
            )}
          </CardContent>
        </Card>

        <Card>
          <CardHeader>
            <CardTitle>Recent Activity</CardTitle>
          </CardHeader>
          <CardContent>
            {statsQuery.isLoading ? (
              <div className="space-y-2">
                {Array.from({ length: 5 }).map((_, i) => (
                  <Skeleton key={i} className="h-8 w-full" />
                ))}
              </div>
            ) : recentActivity.length === 0 ? (
              <p className="text-sm text-muted-foreground">No activity yet.</p>
            ) : (
              <Table>
                <TableHeader>
                  <TableRow>
                    <TableHead>Type</TableHead>
                    <TableHead>Detail</TableHead>
                    <TableHead>Time</TableHead>
                  </TableRow>
                </TableHeader>
                <TableBody>
                  {recentActivity.map((item, i) => (
                    <TableRow key={i}>
                      <TableCell>
                        <Badge
                          variant={
                            item.type === "post.created"
                              ? "default"
                              : "secondary"
                          }
                        >
                          {item.type === "post.created"
                            ? "Post"
                            : item.type === "comment.created"
                              ? "Comment"
                              : item.type}
                        </Badge>
                      </TableCell>
                      <TableCell className="max-w-[200px] truncate">
                        {item.type === "post.created" ? (
                          <Link
                            href={`/admin/posts/${item.slug}/edit`}
                            className="hover:underline"
                          >
                            {item.title}
                          </Link>
                        ) : (
                          <span className="text-muted-foreground">
                            {item.content?.slice(0, 60)}
                            {(item.content?.length ?? 0) > 60 ? "..." : ""}
                          </span>
                        )}
                      </TableCell>
                      <TableCell className="text-muted-foreground text-sm">
                        {new Date(item.at).toLocaleDateString()}
                      </TableCell>
                    </TableRow>
                  ))}
                </TableBody>
              </Table>
            )}
          </CardContent>
        </Card>
      </div>

      {isAdmin() && overview?.content_by_type && Object.keys(overview.content_by_type).length > 0 && (
        <Card>
          <CardHeader>
            <CardTitle>Content Types</CardTitle>
          </CardHeader>
          <CardContent>
            <div className="grid gap-3 sm:grid-cols-2 lg:grid-cols-4">
              {Object.entries(overview.content_by_type).map(([table, count]) => (
                <Link key={table} href={`/admin/content-types`}>
                  <div className="flex items-center justify-between rounded-lg border p-3 hover:bg-muted/50 transition-colors">
                    <span className="text-sm font-medium">{table}</span>
                    <span className="text-lg font-bold">{count}</span>
                  </div>
                </Link>
              ))}
            </div>
          </CardContent>
        </Card>
      )}
    </div>
  );
}
