"use client";

import { useState } from "react";
import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import { MessageCircle, Reply, Send } from "lucide-react";
import { toast } from "sonner";
import { Button } from "@/components/ui/button";
import { Textarea } from "@/components/ui/textarea";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Skeleton } from "@/components/ui/skeleton";
import { Separator } from "@/components/ui/separator";
import type { Comment } from "@/lib/types";
import { client } from "@/lib/raisfast";
import type { PaginatedData } from "@raisfast/sdk";
import { useAuthStore } from "@/stores/auth";

interface CommentSectionProps {
  postSlug: string;
}

function formatDate(dateStr: string): string {
  return new Date(dateStr).toLocaleDateString("en-US", {
    year: "numeric",
    month: "short",
    day: "numeric",
    hour: "2-digit",
    minute: "2-digit",
  });
}

function CommentItem({
  comment,
  depth = 0,
  onReply,
}: {
  comment: Comment;
  depth?: number;
  onReply: (parentId: string) => void;
}) {
  return (
    <div className={depth > 0 ? "ml-6 border-l-2 border-muted pl-4" : ""}>
      <div className="space-y-1 py-3">
        <div className="flex items-center gap-2 text-sm">
          <span className="font-medium">{comment.author_name || comment.nickname}</span>
          <span className="text-muted-foreground">{formatDate(comment.created_at)}</span>
        </div>
        <p className="text-sm leading-relaxed">{comment.content}</p>
        <Button
          variant="ghost"
          size="sm"
          className="h-7 gap-1 text-xs text-muted-foreground"
          onClick={() => onReply(comment.id)}
        >
          <Reply className="h-3 w-3" />
          Reply
        </Button>
      </div>
      {comment.replies?.map((reply) => (
        <CommentItem key={reply.id} comment={reply} depth={depth + 1} onReply={onReply} />
      ))}
    </div>
  );
}

function CommentForm({
  postSlug,
  parentId,
  onCancel,
}: {
  postSlug: string;
  parentId?: string;
  onCancel?: () => void;
}) {
  const queryClient = useQueryClient();
  const { user } = useAuthStore();
  const isLoggedIn = useAuthStore((s) => s.isLoggedIn());
  const [content, setContent] = useState("");
  const [nickname, setNickname] = useState("");
  const [email, setEmail] = useState("");

  const mutation = useMutation({
    mutationFn: (data: {
      content: string;
      parent_id?: string;
      nickname?: string;
      email?: string;
    }) => {
      const endpoint = isLoggedIn
        ? `/posts/${postSlug}/comments/authed`
        : `/posts/${postSlug}/comments`;
      return client.send(endpoint, { method: "POST", body: data });
    },
    onSuccess: () => {
      toast.success("Comment posted");
      setContent("");
      setNickname("");
      setEmail("");
      queryClient.invalidateQueries({ queryKey: ["comments", postSlug] });
      onCancel?.();
    },
    onError: () => {
      toast.error("Failed to post comment");
    },
  });

  function handleSubmit(e: React.FormEvent) {
    e.preventDefault();
    if (!content.trim()) return;
    const data: Record<string, string> = { content };
    if (parentId) data.parent_id = parentId;
    if (!isLoggedIn) {
      data.nickname = nickname;
      data.email = email;
    }
    mutation.mutate(data as Parameters<typeof mutation.mutate>[0]);
  }

  return (
    <form onSubmit={handleSubmit} className="space-y-3">
      {!isLoggedIn && (
        <div className="grid gap-3 sm:grid-cols-2">
          <div className="space-y-1">
            <Label htmlFor="nickname">Nickname</Label>
            <Input
              id="nickname"
              value={nickname}
              onChange={(e) => setNickname(e.target.value)}
              placeholder="Your name"
              required
            />
          </div>
          <div className="space-y-1">
            <Label htmlFor="email">Email</Label>
            <Input
              id="email"
              type="email"
              value={email}
              onChange={(e) => setEmail(e.target.value)}
              placeholder="you@example.com"
              required
            />
          </div>
        </div>
      )}
      <div className="space-y-1">
        <Label htmlFor="content">Comment</Label>
        <Textarea
          id="content"
          value={content}
          onChange={(e) => setContent(e.target.value)}
          placeholder={isLoggedIn ? "Write a comment..." : "Write a comment as guest..."}
          rows={4}
          required
        />
      </div>
      <div className="flex gap-2">
        <Button type="submit" disabled={mutation.isPending} size="sm">
          <Send className="mr-1.5 h-3.5 w-3.5" />
          {mutation.isPending ? "Posting..." : "Post"}
        </Button>
        {onCancel && (
          <Button type="button" variant="ghost" size="sm" onClick={onCancel}>
            Cancel
          </Button>
        )}
      </div>
    </form>
  );
}

export function CommentSection({ postSlug }: CommentSectionProps) {
  const [replyTo, setReplyTo] = useState<string | null>(null);

  const { data: comments, isLoading } = useQuery<Comment[]>({
    queryKey: ["comments", postSlug],
    queryFn: async () => {
      const res = await client.send<PaginatedData<Comment>>(
        `/posts/${postSlug}/comments`
      );
      return res.items;
    },
  });

  return (
    <section className="space-y-6">
      <div className="flex items-center gap-2">
        <MessageCircle className="h-5 w-5" />
        <h3 className="text-lg font-semibold">
          Comments {comments ? `(${comments.length})` : ""}
        </h3>
      </div>

      <div className="space-y-4 rounded-lg border p-4">
        <p className="text-sm font-medium">Leave a comment</p>
        <CommentForm postSlug={postSlug} />
      </div>

      <Separator />

      {isLoading ? (
        <div className="space-y-4">
          {Array.from({ length: 3 }).map((_, i) => (
            <div key={i} className="space-y-2">
              <Skeleton className="h-4 w-32" />
              <Skeleton className="h-4 w-full" />
              <Skeleton className="h-4 w-2/3" />
            </div>
          ))}
        </div>
      ) : comments && comments.length > 0 ? (
        <div className="divide-y">
          {comments.map((comment) => (
            <CommentItem
              key={comment.id}
              comment={comment}
              onReply={(id) => setReplyTo(id)}
            />
          ))}
        </div>
      ) : (
        <p className="py-8 text-center text-sm text-muted-foreground">
          No comments yet. Be the first!
        </p>
      )}

      {replyTo && (
        <div className="rounded-lg border bg-muted/50 p-4">
          <p className="mb-3 text-sm text-muted-foreground">
            Replying to comment
          </p>
          <CommentForm
            postSlug={postSlug}
            parentId={replyTo}
            onCancel={() => setReplyTo(null)}
          />
        </div>
      )}
    </section>
  );
}
