"use client";

import React, { useState, useEffect } from "react";
import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import Link from "next/link";
import {
  ArrowLeft,
  Eye,
  MessageSquare,
  Pin,
  Lock,
  CheckCircle2,
  ThumbsUp,
  ThumbsDown,
  Send,
  Reply,
  BarChart3,
  Trash2,
  PinOff,
  Unlock,
  Pencil,
} from "lucide-react";
import { toast } from "sonner";
import { useRouter } from "next/navigation";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { Separator } from "@/components/ui/separator";
import { Skeleton } from "@/components/ui/skeleton";
import { Card, CardContent } from "@/components/ui/card";
import { RichTextEditor } from "@/components/editor/rich-text-editor";
import { client } from "@/lib/raisfast";
import { forum, type ForumTopic, type ForumReply, type PaginatedResult, type Poll } from "@/lib/forum";
import { useAuthStore } from "@/stores/auth";
import { PostContent } from "@/components/blog/post-content";

function formatDate(dateStr: string): string {
  return new Date(dateStr).toLocaleDateString("en-US", {
    year: "numeric",
    month: "short",
    day: "numeric",
    hour: "2-digit",
    minute: "2-digit",
  });
}

function ReplyItem({
  reply,
  topic,
  onReply,
}: {
  reply: ForumReply;
  topic: ForumTopic;
  onReply: (id: string) => void;
}) {
  const queryClient = useQueryClient();
  const { user } = useAuthStore();
  const isLoggedIn = useAuthStore((s) => s.isLoggedIn());

  const voteMut = useMutation({
    mutationFn: (value: number) =>
      forum.vote(user!.id, "reply", reply.id, value),
    onSuccess: () => queryClient.invalidateQueries({ queryKey: ["forum-topic", topic.id] }),
  });

  const acceptMut = useMutation({
    mutationFn: () => forum.acceptAnswer(user!.id, reply.id),
    onSuccess: () => {
      toast.success("Answer accepted");
      queryClient.invalidateQueries({ queryKey: ["forum-topic", topic.id] });
    },
    onError: () => toast.error("Failed to accept answer"),
  });

  const deleteReplyMut = useMutation({
    mutationFn: () => forum.deleteReply(reply.id),
    onSuccess: () => {
      toast.success("Reply deleted");
      queryClient.invalidateQueries({ queryKey: ["forum-topic", topic.id] });
    },
    onError: () => toast.error("Failed to delete reply"),
  });

  const isAdmin = useAuthStore((s) => s.isAdmin());
  const canDeleteReply = isAdmin || reply.author_id === user?.id;

  return (
    <div className={`space-y-2 py-4 ${reply.is_answer ? "rounded-lg border-2 border-green-500/30 bg-green-500/5 p-4" : ""}`}>
      <div className="flex items-start gap-3">
        <div className="flex h-9 w-9 shrink-0 items-center justify-center rounded-full bg-muted text-xs font-medium">
          {(reply.author_name || "U").charAt(0).toUpperCase()}
        </div>
        <div className="min-w-0 flex-1">
          <div className="flex items-center gap-2 text-sm">
            <span className="font-medium">{reply.author_name || "User"}</span>
            <span className="text-muted-foreground">{formatDate(reply.created_at)}</span>
            {reply.is_answer && (
              <Badge variant="outline" className="border-green-500 text-green-600">
                <CheckCircle2 className="mr-1 h-3 w-3" />
                Best Answer
              </Badge>
            )}
          </div>
          <div className="prose prose-sm mt-2 max-w-none dark:prose-invert">
            <PostContent content={reply.content} />
          </div>
          <div className="mt-2 flex items-center gap-2">
            <Button
              variant="ghost"
              size="sm"
              className="h-7 gap-1 text-xs"
              onClick={() => {
                if (!isLoggedIn) return toast.error("Please login to vote");
                voteMut.mutate(1);
              }}
              disabled={voteMut.isPending}
            >
              <ThumbsUp className="h-3 w-3" />
              {reply.vote_count > 0 ? reply.vote_count : ""}
            </Button>
            <Button
              variant="ghost"
              size="sm"
              className="h-7 text-xs"
              onClick={() => {
                if (!isLoggedIn) return toast.error("Please login to vote");
                voteMut.mutate(-1);
              }}
              disabled={voteMut.isPending}
            >
              <ThumbsDown className="h-3 w-3" />
            </Button>
            {isLoggedIn && (
              <Button
                variant="ghost"
                size="sm"
                className="h-7 gap-1 text-xs"
                onClick={() => onReply(reply.id)}
              >
                <Reply className="h-3 w-3" />
                Reply
              </Button>
            )}
            {isLoggedIn && topic.author_id === user!.id && !topic.is_solved && !reply.is_answer && (
              <Button
                variant="outline"
                size="sm"
                className="h-7 gap-1 text-xs text-green-600"
                onClick={() => acceptMut.mutate()}
                disabled={acceptMut.isPending}
              >
                <CheckCircle2 className="h-3 w-3" />
                Accept
              </Button>
            )}
            {canDeleteReply && (
              <Button
                variant="ghost"
                size="sm"
                className="h-7 gap-1 text-xs text-destructive hover:text-destructive"
                onClick={() => { if (confirm("Delete this reply?")) deleteReplyMut.mutate(); }}
                disabled={deleteReplyMut.isPending}
              >
                <Trash2 className="h-3 w-3" />
                Delete
              </Button>
            )}
          </div>
        </div>
      </div>
    </div>
  );
}

function ReplyForm({
  topicId,
  parentId,
  onCancel,
}: {
  topicId: string;
  parentId?: string;
  onCancel?: () => void;
}) {
  const queryClient = useQueryClient();
  const { user } = useAuthStore();
  const [content, setContent] = useState("");

  const mutation = useMutation({
    mutationFn: () =>
      forum.createReply({
        content,
        topic: topicId,
        author_id: user!.id,
        parent_reply: parentId,
      }),
    onSuccess: () => {
      toast.success("Reply posted");
      setContent("");
      queryClient.invalidateQueries({ queryKey: ["forum-topic", topicId] });
      onCancel?.();
    },
    onError: () => toast.error("Failed to post reply"),
  });

  function handleSubmit(e: React.FormEvent) {
    e.preventDefault();
    if (!content.trim()) return;
    mutation.mutate();
  }

  return (
    <form onSubmit={handleSubmit} className="space-y-3">
      <RichTextEditor
        markdown={content}
        onChange={setContent}
        placeholder="Write your reply..."
      />
      <div className="flex gap-2">
        <Button type="submit" size="sm" disabled={mutation.isPending}>
          <Send className="mr-1.5 h-3.5 w-3.5" />
          {mutation.isPending ? "Posting..." : "Post Reply"}
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

function PollWidget({ topicId, isAuthor }: { topicId: string; isAuthor: boolean }) {
  const queryClient = useQueryClient();
  const { user } = useAuthStore();
  const isLoggedIn = useAuthStore((s) => s.isLoggedIn());
  const [selected, setSelected] = useState<Set<string>>(new Set());

  const { data: poll, isLoading } = useQuery({
    queryKey: ["forum-poll", topicId],
    queryFn: () => forum.getPoll(topicId, user?.id),
    enabled: !!topicId,
  });

  const castVoteMut = useMutation({
    mutationFn: () => {
      if (!user || !poll) throw new Error("not available");
      return forum.castVote(user.id, poll.id, Array.from(selected));
    },
    onSuccess: () => {
      toast.success("Vote submitted");
      setSelected(new Set());
      queryClient.invalidateQueries({ queryKey: ["forum-poll", topicId] });
    },
    onError: (err) => toast.error(err.message || "Failed to vote"),
  });

  const deletePollMut = useMutation({
    mutationFn: () => {
      if (!user || !poll) throw new Error("not available");
      return forum.deletePoll(user.id, poll.id);
    },
    onSuccess: () => {
      toast.success("Poll deleted");
      queryClient.invalidateQueries({ queryKey: ["forum-poll", topicId] });
    },
    onError: () => toast.error("Failed to delete poll"),
  });

  if (isLoading) return <Skeleton className="h-40 w-full" />;
  if (!poll) return null;

  const maxChoices = poll.max_choices || 1;
  const hasVoted = poll.user_votes && poll.user_votes.length > 0;
  const canVote = isLoggedIn && !hasVoted && !poll.is_closed;

  function toggleOption(optId: string) {
    const next = new Set(selected);
    if (next.has(optId)) {
      next.delete(optId);
    } else if (next.size < maxChoices) {
      next.add(optId);
    }
    setSelected(next);
  }

  return (
    <div className="rounded-lg border p-4 space-y-3">
      <div className="flex items-center justify-between">
        <h3 className="flex items-center gap-2 text-sm font-semibold">
          <BarChart3 className="h-4 w-4" />
          {poll.question}
        </h3>
        {poll.is_closed && (
          <Badge variant="outline" className="text-yellow-600 border-yellow-500">Closed</Badge>
        )}
      </div>

      <div className="space-y-2">
        {poll.options.map((opt) => {
          const pct = poll.total_votes > 0 ? Math.round(((opt.vote_count || 0) / poll.total_votes) * 100) : 0;
          const isVoted = poll.user_votes.includes(opt.id);
          const isSelected = selected.has(opt.id);

          return (
            <button
              key={opt.id}
              type="button"
              disabled={!canVote}
              onClick={() => toggleOption(opt.id)}
              className={`relative w-full rounded-md border px-3 py-2 text-left text-sm transition-colors
                ${canVote && isSelected ? "border-primary bg-primary/10" : ""}
                ${canVote && !isSelected ? "hover:bg-accent" : ""}
                ${!canVote ? "cursor-default" : "cursor-pointer"}
                ${isVoted ? "border-primary/50" : ""}
              `}
            >
              {hasVoted && (
                <div
                  className="absolute inset-y-0 left-0 rounded-md bg-primary/10"
                  style={{ width: `${pct}%` }}
                />
              )}
              <div className="relative flex items-center justify-between">
                <div className="flex items-center gap-2">
                  {canVote && (
                    <span className={`flex h-4 w-4 items-center justify-center rounded-sm border ${isSelected ? "border-primary bg-primary text-primary-foreground" : "border-muted-foreground/40"}`}>
                      {isSelected && <CheckCircle2 className="h-3 w-3" />}
                    </span>
                  )}
                  {isVoted && !canVote && <CheckCircle2 className="h-3.5 w-3.5 text-primary" />}
                  <span>{opt.text}</span>
                </div>
                {hasVoted && (
                  <span className="text-xs text-muted-foreground">
                    {opt.vote_count} ({pct}%)
                  </span>
                )}
              </div>
            </button>
          );
        })}
      </div>

      <div className="flex items-center justify-between text-xs text-muted-foreground">
        <span>{poll.total_votes} vote{poll.total_votes !== 1 ? "s" : ""}</span>
        {poll.max_choices > 1 && <span>You can select up to {poll.max_choices} options</span>}
      </div>

      <div className="flex items-center gap-2">
        {canVote && (
          <Button
            size="sm"
            className="h-7 text-xs"
            disabled={selected.size === 0 || castVoteMut.isPending}
            onClick={() => castVoteMut.mutate()}
          >
            {castVoteMut.isPending ? "Voting..." : "Vote"}
          </Button>
        )}
        {!isLoggedIn && !hasVoted && (
          <span className="text-xs text-muted-foreground">
            <Link href="/auth/login" className="text-primary hover:underline">Login</Link> to vote
          </span>
        )}
        {isAuthor && !poll.is_closed && (
          <Button
            variant="ghost"
            size="sm"
            className="h-7 text-xs text-destructive hover:text-destructive"
            disabled={deletePollMut.isPending}
            onClick={() => deletePollMut.mutate()}
          >
            Delete Poll
          </Button>
        )}
      </div>
    </div>
  );
}

export default function TopicDetailPage({ params }: { params: Promise<{ id: string }> }) {
  const { id } = React.use(params);
  const router = useRouter();
  const queryClient = useQueryClient();
  const isLoggedIn = useAuthStore((s) => s.isLoggedIn());
  const { user } = useAuthStore();
  const [replyTo, setReplyTo] = useState<string | null>(null);
  const [mounted, setMounted] = useState(false);
  useEffect(() => setMounted(true), []);

  const { data: topic, isLoading } = useQuery({
    queryKey: ["forum-topic", id],
    queryFn: () => forum.getTopic(id),
  });

  const topicVoteMut = useMutation({
    mutationFn: (value: number) =>
      forum.vote(user!.id, "topic", id, value),
    onSuccess: () => queryClient.invalidateQueries({ queryKey: ["forum-topic", id] }),
  });

  const isAdmin = useAuthStore((s) => s.isAdmin());
  const isAuthor = topic?.author_id === user?.id;
  const canManage = isAdmin || isAuthor;

  const pinMut = useMutation({
    mutationFn: (pinned: boolean) => forum.updateTopic(id, { is_pinned: pinned }),
    onSuccess: () => { toast.success("Updated"); queryClient.invalidateQueries({ queryKey: ["forum-topic", id] }); },
    onError: () => toast.error("Failed"),
  });

  const lockMut = useMutation({
    mutationFn: (locked: boolean) => forum.updateTopic(id, { is_locked: locked }),
    onSuccess: () => { toast.success("Updated"); queryClient.invalidateQueries({ queryKey: ["forum-topic", id] }); },
    onError: () => toast.error("Failed"),
  });

  const deleteMut = useMutation({
    mutationFn: () => forum.deleteTopic(id),
    onSuccess: () => { toast.success("Deleted"); router.push("/forum"); },
    onError: () => toast.error("Failed to delete"),
  });

  if (isLoading) {
    return (
      <div className="space-y-6">
        <Skeleton className="h-6 w-20" />
        <Skeleton className="h-8 w-3/4" />
        <Skeleton className="h-4 w-1/2" />
        <Skeleton className="h-40 w-full" />
      </div>
    );
  }

  if (!topic) {
    return (
      <p className="py-16 text-center text-muted-foreground">Topic not found</p>
    );
  }

  return (
    <div className="space-y-6">
      <div className="flex items-center gap-3">
        <Link href={topic.board_slug ? `/forum/${topic.board_slug}` : "/forum"}>
          <Button variant="ghost" size="sm">
            <ArrowLeft className="mr-1 h-4 w-4" />
            Back
          </Button>
        </Link>
        {topic.board_name && (
          <Badge variant="outline">{topic.board_name}</Badge>
        )}
      </div>

      <div>
        <div className="flex items-center gap-2">
          {topic.is_pinned === true && <Pin className="h-4 w-4 text-orange-500" />}
          {topic.is_locked === true && <Lock className="h-4 w-4 text-yellow-500" />}
          {topic.is_solved === true && <CheckCircle2 className="h-4 w-4 text-green-500" />}
          <h1 className="text-xl font-bold">{topic.title}</h1>
        </div>
        <div className="mt-2 flex items-center gap-3 text-sm text-muted-foreground">
          <span className="font-medium">{topic.author_name || "User"}</span>
          <span>{formatDate(topic.created_at)}</span>
          <span className="flex items-center gap-1">
            <Eye className="h-3.5 w-3.5" />
            {topic.view_count}
          </span>
          <span className="flex items-center gap-1">
            <MessageSquare className="h-3.5 w-3.5" />
            {topic.reply_count}
          </span>
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

      <Card>
        <CardContent className="p-6">
          <div className="prose max-w-none dark:prose-invert">
            <PostContent content={topic.content || ""} />
          </div>
          <div className="mt-4 flex items-center gap-2">
            <Button
              variant="ghost"
              size="sm"
              className="h-7 gap-1 text-xs"
              onClick={() => {
                if (!isLoggedIn) return toast.error("Please login to vote");
                topicVoteMut.mutate(1);
              }}
              disabled={topicVoteMut.isPending}
            >
              <ThumbsUp className="h-3 w-3" />
            </Button>
            <Button
              variant="ghost"
              size="sm"
              className="h-7 text-xs"
              onClick={() => {
                if (!isLoggedIn) return toast.error("Please login to vote");
                topicVoteMut.mutate(-1);
              }}
              disabled={topicVoteMut.isPending}
            >
              <ThumbsDown className="h-3 w-3" />
            </Button>
          </div>
          {canManage && mounted && (
            <div className="mt-3 flex items-center gap-1.5 border-t pt-3">
              <Link href={`/forum/topic/${id}/edit`}>
                <Button variant="outline" size="sm" className="h-7 gap-1 text-xs">
                  <Pencil className="h-3 w-3" />
                  Edit
                </Button>
              </Link>
              {isAdmin && (
                <Button
                  variant="outline"
                  size="sm"
                  className="h-7 gap-1 text-xs"
                  onClick={() => pinMut.mutate(!topic.is_pinned)}
                  disabled={pinMut.isPending}
                >
                  {topic.is_pinned ? <PinOff className="h-3 w-3" /> : <Pin className="h-3 w-3" />}
                  {topic.is_pinned ? "Unpin" : "Pin"}
                </Button>
              )}
              {isAdmin && (
                <Button
                  variant="outline"
                  size="sm"
                  className="h-7 gap-1 text-xs"
                  onClick={() => lockMut.mutate(!topic.is_locked)}
                  disabled={lockMut.isPending}
                >
                  {topic.is_locked ? <Unlock className="h-3 w-3" /> : <Lock className="h-3 w-3" />}
                  {topic.is_locked ? "Unlock" : "Lock"}
                </Button>
              )}
              <Button
                variant="outline"
                size="sm"
                className="h-7 gap-1 text-xs text-destructive hover:text-destructive"
                onClick={() => { if (confirm("Delete this topic?")) deleteMut.mutate(); }}
                disabled={deleteMut.isPending}
              >
                <Trash2 className="h-3 w-3" />
                Delete
              </Button>
            </div>
          )}
        </CardContent>
      </Card>

      {mounted && <PollWidget topicId={id} isAuthor={topic.author_id === user?.id} />}

      <Separator />

      <div className="flex items-center gap-2">
        <MessageSquare className="h-5 w-5" />
        <h2 className="text-lg font-semibold">
          Replies {topic.reply_count > 0 ? `(${topic.reply_count})` : ""}
        </h2>
      </div>

      {mounted && isLoggedIn && !topic.is_locked && (
        <div className="rounded-lg border p-4">
          <p className="mb-3 text-sm font-medium">Post a reply</p>
          <ReplyForm topicId={id} />
        </div>
      )}

      {mounted && !isLoggedIn && (
        <div className="rounded-lg border bg-muted/50 p-4 text-center text-sm text-muted-foreground">
          <Link href="/auth/login" className="text-primary hover:underline">
            Login
          </Link>{" "}
          to join the discussion.
        </div>
      )}

      {topic.is_locked && (
        <div className="rounded-lg border border-yellow-500/30 bg-yellow-500/5 p-4 text-center text-sm text-yellow-700 dark:text-yellow-400">
          <Lock className="mr-1.5 inline h-4 w-4" />
          This topic is locked. No new replies can be posted.
        </div>
      )}

      {topic.reply_count > 0 ? (
        <RepliesList topicId={id} topic={topic} onReply={(rid) => setReplyTo(rid)} />
      ) : (
        !topic.is_locked && (
          <p className="py-8 text-center text-sm text-muted-foreground">
            No replies yet. Be the first to respond!
          </p>
        )
      )}

      {replyTo && mounted && isLoggedIn && (
        <div className="rounded-lg border bg-muted/50 p-4">
          <p className="mb-3 text-sm text-muted-foreground">
            Replying to comment
          </p>
          <ReplyForm
            topicId={id}
            parentId={replyTo}
            onCancel={() => setReplyTo(null)}
          />
        </div>
      )}
    </div>
  );
}

function RepliesList({
  topicId,
  topic,
  onReply,
}: {
  topicId: string;
  topic: ForumTopic;
  onReply: (id: string) => void;
}) {
  const { data, isLoading } = useQuery({
    queryKey: ["forum-replies", topicId],
    queryFn: async () => {
      return client.send<PaginatedResult<ForumReply>>(`/cms/forum_replies?page_size=100&topic=${topicId}`);
    },
  });

  if (isLoading) {
    return (
      <div className="space-y-4">
        {Array.from({ length: 3 }).map((_, i) => (
          <div key={i} className="space-y-2">
            <Skeleton className="h-4 w-32" />
            <Skeleton className="h-4 w-full" />
            <Skeleton className="h-4 w-2/3" />
          </div>
        ))}
      </div>
    );
  }

  const replies = data?.items || [];

  if (replies.length === 0) return null;

  return (
    <div className="divide-y">
      {replies.map((reply) => (
        <ReplyItem
          key={reply.id}
          reply={reply}
          topic={topic}
          onReply={onReply}
        />
      ))}
    </div>
  );
}
