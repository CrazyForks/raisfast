"use client";

import React, { Suspense } from "react";
import { useQuery } from "@tanstack/react-query";
import { useSearchParams } from "next/navigation";
import Link from "next/link";
import { MessageSquare, Eye, Pin, Lock, CheckCircle2, ArrowLeft } from "lucide-react";
import { Card, CardContent } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { Skeleton } from "@/components/ui/skeleton";
import { Button } from "@/components/ui/button";
import { Pagination } from "@/components/common/pagination";
import { forum, type ForumTopic } from "@/lib/forum";
import { useAuthStore } from "@/stores/auth";

function formatDate(dateStr: string): string {
  return new Date(dateStr).toLocaleDateString("en-US", {
    month: "short",
    day: "numeric",
    hour: "2-digit",
    minute: "2-digit",
  });
}

function TopicRow({ topic }: { topic: ForumTopic }) {
  return (
    <Card className="transition-shadow hover:shadow-sm">
      <CardContent className="flex items-center gap-4 p-4">
        <div className="flex h-10 w-10 shrink-0 items-center justify-center rounded-full bg-muted">
          <MessageSquare className="h-5 w-5 text-muted-foreground" />
        </div>
        <div className="min-w-0 flex-1">
          <div className="flex items-center gap-2">
            {topic.is_pinned && (
              <Pin className="h-3.5 w-3.5 text-orange-500" />
            )}
            {topic.is_locked && (
              <Lock className="h-3.5 w-3.5 text-yellow-500" />
            )}
            {topic.is_solved && (
              <CheckCircle2 className="h-3.5 w-3.5 text-green-500" />
            )}
            <Link
              href={`/forum/topic/${topic.id}`}
              className="font-medium hover:underline"
            >
              {topic.title}
            </Link>
          </div>
          <div className="mt-1 flex items-center gap-3 text-xs text-muted-foreground">
            <span>{formatDate(topic.created_at)}</span>
            {topic.tags && (
              <div className="flex gap-1">
                {topic.tags.split(",").map((tag: string) => (
                  <Badge key={tag} variant="outline" className="px-1.5 py-0 text-xs">
                    {tag.trim()}
                  </Badge>
                ))}
              </div>
            )}
          </div>
        </div>
        <div className="flex shrink-0 items-center gap-4 text-sm text-muted-foreground">
          <span className="flex items-center gap-1" title="Replies">
            <MessageSquare className="h-3.5 w-3.5" />
            {topic.reply_count}
          </span>
          <span className="flex items-center gap-1" title="Views">
            <Eye className="h-3.5 w-3.5" />
            {topic.view_count}
          </span>
        </div>
      </CardContent>
    </Card>
  );
}

function BoardTopicsContent({ slug }: { slug: string }) {
  const searchParams = useSearchParams();
  const page = Number(searchParams.get("page") ?? "1");
  const isLoggedIn = useAuthStore((s) => s.isLoggedIn());

  const { data, isLoading } = useQuery({
    queryKey: ["board-topics", slug, page],
    queryFn: () => forum.listBoardTopics(slug, page),
  });

  return (
    <div className="space-y-6">
      <div className="flex items-center justify-between">
        <div className="flex items-center gap-3">
          <Link href="/forum">
            <Button variant="ghost" size="sm">
              <ArrowLeft className="mr-1 h-4 w-4" />
              All Boards
            </Button>
          </Link>
        </div>
        {isLoggedIn && data?.board_id && (
          <Link href={`/forum/new?board_id=${data.board_id}`}>
            <Button size="sm">New Topic</Button>
          </Link>
        )}
      </div>

      {isLoading ? (
        <div className="space-y-3">
          {Array.from({ length: 8 }).map((_, i) => (
            <Card key={i}>
              <CardContent className="flex items-center gap-4 p-4">
                <Skeleton className="h-10 w-10 rounded-full" />
                <div className="flex-1 space-y-2">
                  <Skeleton className="h-4 w-60" />
                  <Skeleton className="h-3 w-40" />
                </div>
              </CardContent>
            </Card>
          ))}
        </div>
      ) : data && data.items.length > 0 ? (
        <>
          <div className="space-y-2">
            {data.items.map((topic) => (
              <TopicRow key={topic.id} topic={topic} />
            ))}
          </div>
          <Pagination page={data.page} pageSize={data.page_size} total={data.total} />
        </>
      ) : (
        <p className="py-16 text-center text-muted-foreground">
          No topics yet. Be the first to start a discussion!
        </p>
      )}
    </div>
  );
}

export default function BoardTopicsPage({ params }: { params: Promise<{ slug: string }> }) {
  return (
    <Suspense>
      <BoardTopicsContentWrapped params={params} />
    </Suspense>
  );
}

function BoardTopicsContentWrapped({ params }: { params: Promise<{ slug: string }> }) {
  const slug = React.use(params);
  return <BoardTopicsContent slug={slug.slug} />;
}
