"use client";

import React, { Suspense } from "react";
import { useQuery } from "@tanstack/react-query";
import { useSearchParams } from "next/navigation";
import Link from "next/link";
import { MessageSquare, Eye, Pin, Lock, CheckCircle2, ArrowLeft } from "lucide-react";
import { Badge } from "@/components/ui/badge";
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
    <Link
      href={`/forum/topic/${topic.id}`}
      className="flex items-center gap-3 px-3 py-2 rounded-md hover:bg-muted/50 transition-colors text-sm"
    >
      <div className="min-w-0 flex-1 flex items-center gap-2">
        {topic.is_pinned === true && <Pin className="h-3 w-3 shrink-0 text-orange-500" />}
        {topic.is_locked === true && <Lock className="h-3 w-3 shrink-0 text-yellow-500" />}
        {topic.is_solved === true && <CheckCircle2 className="h-3 w-3 shrink-0 text-green-500" />}
        <span className="truncate font-medium">{topic.title}</span>
        {topic.tags && (
          <div className="hidden sm:flex shrink-0 gap-1">
            {topic.tags.split(",").slice(0, 2).map((tag: string) => (
              <Badge key={tag} variant="outline" className="px-1.5 py-0 text-xs">
                {tag.trim()}
              </Badge>
            ))}
          </div>
        )}
      </div>
          <span className="hidden sm:flex shrink-0 items-center gap-1 w-28 justify-center text-xs text-muted-foreground">
        {formatDate(topic.created_at)}
      </span>
      <span className="shrink-0 flex items-center gap-1 w-14 justify-center text-xs text-muted-foreground">
        <MessageSquare className="h-3 w-3" />
        {topic.reply_count}
      </span>
      <span className="shrink-0 flex items-center gap-1 w-14 justify-center text-xs text-muted-foreground">
        <Eye className="h-3 w-3" />
        {topic.view_count}
      </span>
    </Link>
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
    <div className="space-y-4">
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

      <div className="rounded-lg border">
        <div className="flex items-center gap-3 px-3 py-1.5 border-b bg-muted/30 text-xs font-medium text-muted-foreground">
          <div className="flex-1">Topic</div>
          <span className="hidden sm:block w-28 text-center">Date</span>
          <span className="w-14 text-center">Replies</span>
          <span className="w-14 text-center">Views</span>
        </div>
        {isLoading ? (
          <div className="py-8 text-center text-muted-foreground text-sm">Loading...</div>
        ) : data && data.items.length > 0 ? (
          <div className="divide-y">
            {data.items.map((topic: ForumTopic) => (
              <TopicRow key={topic.id} topic={topic} />
            ))}
          </div>
        ) : (
          <div className="py-8 text-center text-muted-foreground text-sm">
            No topics yet. Be the first to start a discussion!
          </div>
        )}
      </div>

      {data && data.items.length > 0 && (
        <Pagination page={data.page} pageSize={data.page_size} total={data.total} />
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
