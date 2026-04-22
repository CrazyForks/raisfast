"use client";

import { useEffect, useState } from "react";
import { useQuery } from "@tanstack/react-query";
import Link from "next/link";
import { MessageSquare, Users, ArrowRight, Pin } from "lucide-react";
import { Card, CardContent } from "@/components/ui/card";
import { Skeleton } from "@/components/ui/skeleton";
import { Badge } from "@/components/ui/badge";
import { forum, type ForumBoard } from "@/lib/forum";
import { useAuthStore } from "@/stores/auth";
import { Button } from "@/components/ui/button";

function BoardCard({ board }: { board: ForumBoard }) {
  const lastActivity = board.last_activity_at
    ? new Date(board.last_activity_at).toLocaleDateString("en-US", {
        month: "short",
        day: "numeric",
        hour: "2-digit",
        minute: "2-digit",
      })
    : "No activity";

  return (
    <Card className="transition-shadow hover:shadow-md">
      <CardContent className="flex items-center gap-4 p-4">
        <div className="flex h-12 w-12 shrink-0 items-center justify-center rounded-lg bg-primary/10 text-primary">
          <MessageSquare className="h-6 w-6" />
        </div>
        <div className="min-w-0 flex-1">
          <div className="flex items-center gap-2">
            <Link
              href={`/forum/${board.slug}`}
              className="font-semibold hover:underline"
            >
              {board.name}
            </Link>
            {board.children && board.children.length > 0 && (
              <div className="flex gap-1">
                {board.children.map((child) => (
                  <Link
                    key={child.id}
                    href={`/forum/${child.slug}`}
                    className="text-xs text-muted-foreground hover:text-primary hover:underline"
                  >
                    {child.name}
                  </Link>
                ))}
              </div>
            )}
          </div>
          {board.description && (
            <p className="truncate text-sm text-muted-foreground">
              {board.description}
            </p>
          )}
          <div className="mt-1 flex items-center gap-4 text-xs text-muted-foreground">
            <span className="flex items-center gap-1">
              <MessageSquare className="h-3 w-3" />
              {board.topic_count} topics
            </span>
            <span className="flex items-center gap-1">
              <Users className="h-3 w-3" />
              {board.post_count} posts
            </span>
            <span>{lastActivity}</span>
          </div>
        </div>
        <Link href={`/forum/${board.slug}`}>
          <ArrowRight className="h-4 w-4 text-muted-foreground" />
        </Link>
      </CardContent>
    </Card>
  );
}

export default function ForumPage() {
  const isLoggedIn = useAuthStore((s) => s.isLoggedIn());
  const [mounted, setMounted] = useState(false);
  useEffect(() => setMounted(true), []);

  const { data: boards, isLoading } = useQuery({
    queryKey: ["forum-boards"],
    queryFn: () => forum.listBoards(),
  });

  return (
    <div className="space-y-6">
      <div className="flex items-center justify-between">
        <div>
          <h1 className="text-2xl font-bold">Forum</h1>
          <p className="text-sm text-muted-foreground">
            Join the discussion, ask questions, and share your thoughts.
          </p>
        </div>
        {mounted && isLoggedIn && (
          <Link href="/forum/new">
            <Button>New Topic</Button>
          </Link>
        )}
      </div>

      {isLoading ? (
        <div className="space-y-3">
          {Array.from({ length: 5 }).map((_, i) => (
            <Card key={i}>
              <CardContent className="flex items-center gap-4 p-4">
                <Skeleton className="h-12 w-12 rounded-lg" />
                <div className="flex-1 space-y-2">
                  <Skeleton className="h-4 w-40" />
                  <Skeleton className="h-3 w-64" />
                </div>
              </CardContent>
            </Card>
          ))}
        </div>
      ) : boards && boards.length > 0 ? (
        <div className="space-y-3">
          {boards.map((board: ForumBoard) => (
            <BoardCard key={board.id} board={board} />
          ))}
        </div>
      ) : (
        <p className="py-16 text-center text-muted-foreground">
          No boards available yet.
        </p>
      )}
    </div>
  );
}
