"use client";

import { useEffect, useState } from "react";
import Link from "next/link";
import { FileText, MessageSquare, Image, Users } from "lucide-react";
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

interface Post {
  id: string;
  title: string;
  slug: string;
  status: string;
  category_name: string | null;
  author_name: string | null;
  created_at: string;
}

interface PaginatedData<T> {
  items: T[];
  total: number;
  page: number;
  page_size: number;
}

export default function DashboardPage() {
  const { isAdmin } = useAuthStore();
  const [mounted, setMounted] = useState(false);

  useEffect(() => {
    setMounted(true);
  }, []);

  const postsQuery = useQuery({
    queryKey: ["admin-posts", 1],
    queryFn: () =>
      api.get<PaginatedData<Post>>("/posts?page=1&page_size=5"),
  });

  const mediaQuery = useQuery({
    queryKey: ["admin-media-count"],
    queryFn: () =>
      api.get<PaginatedData<unknown>>("/media?page=1&page_size=1"),
  });

  const usersQuery = useQuery({
    queryKey: ["admin-users-count"],
    queryFn: () =>
      api.get<PaginatedData<unknown>>("/users?page=1&page_size=1"),
    enabled: isAdmin(),
  });

  const commentsQuery = useQuery({
    queryKey: ["admin-comments-count"],
    queryFn: () =>
      api.get<PaginatedData<unknown>>("/comments?page=1&page_size=1"),
    enabled: isAdmin(),
  });

  const postCount = postsQuery.data?.total ?? 0;
  const recentPosts = postsQuery.data?.items ?? [];

  const stats = [
    {
      label: "Posts",
      value: postsQuery.isLoading ? null : postCount,
      icon: FileText,
      href: "/admin/posts",
    },
    {
      label: "Comments",
      value: commentsQuery.isLoading ? null : (commentsQuery.data?.total ?? 0),
      icon: MessageSquare,
      href: "/admin/comments",
    },
    {
      label: "Media",
      value: mediaQuery.isLoading ? null : (mediaQuery.data?.total ?? 0),
      icon: Image,
      href: "/admin/media",
    },
    ...(isAdmin()
      ? [
          {
            label: "Users",
            value: usersQuery.isLoading ? null : (usersQuery.data?.total ?? 0),
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
        <div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-4">
          {Array.from({ length: 4 }).map((_, i) => (
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

      <div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-4">
        {stats.map((stat) => (
          <Link key={stat.label} href={stat.href}>
            <Card className="hover:bg-muted/50 transition-colors cursor-pointer">
              <CardHeader className="flex flex-row items-center justify-between pb-2">
                <CardTitle className="text-sm font-medium">
                  {stat.label}
                </CardTitle>
                <stat.icon className="size-4 text-muted-foreground" />
              </CardHeader>
              <CardContent>
                {stat.value === null ? (
                  <Skeleton className="h-8 w-16" />
                ) : (
                  <div className="text-2xl font-bold">{stat.value}</div>
                )}
              </CardContent>
            </Card>
          </Link>
        ))}
      </div>

      <Card>
        <CardHeader>
          <div className="flex items-center justify-between">
            <CardTitle>Recent Posts</CardTitle>
            <Link href="/admin/posts">
              <Button variant="outline" size="sm">View All</Button>
            </Link>
          </div>
        </CardHeader>
        <CardContent>
          {postsQuery.isLoading ? (
            <div className="space-y-2">
              {Array.from({ length: 3 }).map((_, i) => (
                <Skeleton key={i} className="h-10 w-full" />
              ))}
            </div>
          ) : recentPosts.length === 0 ? (
            <p className="text-sm text-muted-foreground">No posts yet.</p>
          ) : (
            <Table>
              <TableHeader>
                <TableRow>
                  <TableHead>Title</TableHead>
                  <TableHead>Status</TableHead>
                  <TableHead>Category</TableHead>
                  <TableHead>Author</TableHead>
                  <TableHead>Created</TableHead>
                </TableRow>
              </TableHeader>
              <TableBody>
                {recentPosts.map((post) => (
                  <TableRow key={post.id}>
                    <TableCell className="font-medium">{post.title}</TableCell>
                    <TableCell>
                      <Badge
                        variant={post.status === "published" ? "default" : "secondary"}
                      >
                        {post.status}
                      </Badge>
                    </TableCell>
                    <TableCell>{post.category_name || "—"}</TableCell>
                    <TableCell>{post.author_name || "—"}</TableCell>
                    <TableCell>
                      {new Date(post.created_at).toLocaleDateString()}
                    </TableCell>
                  </TableRow>
                ))}
              </TableBody>
            </Table>
          )}
        </CardContent>
      </Card>
    </div>
  );
}
