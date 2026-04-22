"use client";

import React, { useState, useEffect } from "react";
import { useQuery, useMutation } from "@tanstack/react-query";
import { useRouter, useSearchParams } from "next/navigation";
import Link from "next/link";
import { ArrowLeft, Send, Plus, X, BarChart3 } from "lucide-react";
import { toast } from "sonner";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Skeleton } from "@/components/ui/skeleton";
import { RichTextEditor } from "@/components/editor/rich-text-editor";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { forum } from "@/lib/forum";
import { useAuthStore } from "@/stores/auth";

export default function NewTopicPage() {
  const router = useRouter();
  const searchParams = useSearchParams();
  const preselectedBoard = searchParams.get("board_id") || "";
  const { user } = useAuthStore();
  const isLoggedIn = useAuthStore((s) => s.isLoggedIn());
  const [mounted, setMounted] = useState(false);
  useEffect(() => setMounted(true), []);

  const [title, setTitle] = useState("");
  const [content, setContent] = useState("");
  const [boardId, setBoardId] = useState(preselectedBoard);
  const [tags, setTags] = useState("");

  const [showPoll, setShowPoll] = useState(false);
  const [pollQuestion, setPollQuestion] = useState("");
  const [pollOptions, setPollOptions] = useState(["", ""]);
  const [pollMaxChoices, setPollMaxChoices] = useState(1);

  const { data: boards, isLoading } = useQuery({
    queryKey: ["forum-boards"],
    queryFn: () => forum.listBoards(),
  });

  const createMut = useMutation({
    mutationFn: async () => {
      const topic = await forum.createTopic({
        title: title.trim(),
        content: content.trim(),
        board: boardId,
        author_id: user!.id,
        tags: tags.trim() || undefined,
      });

      if (showPoll && pollQuestion.trim() && pollOptions.filter((o) => o.trim()).length >= 2) {
        await forum.createPoll(user!.id, {
          topic_id: topic.id,
          question: pollQuestion.trim(),
          options: pollOptions.filter((o) => o.trim()),
          max_choices: pollMaxChoices,
        });
      }

      return topic;
    },
    onSuccess: (data) => {
      toast.success("Topic created");
      router.push(`/forum/topic/${data.id}`);
    },
    onError: () => toast.error("Failed to create topic"),
  });

  if (!mounted || !isLoggedIn) {
    return (
      <div className="py-16 text-center text-muted-foreground">
        <Link href="/auth/login" className="text-primary hover:underline">
          Login
        </Link>{" "}
        to create a topic.
      </div>
    );
  }

  function handleSubmit(e: React.FormEvent) {
    e.preventDefault();
    if (!title.trim() || !content.trim() || !boardId) {
      toast.error("Please fill in all required fields");
      return;
    }
    if (showPoll && pollOptions.filter((o) => o.trim()).length < 2) {
      toast.error("Poll needs at least 2 options");
      return;
    }
    createMut.mutate();
  }

  function addPollOption() {
    if (pollOptions.length >= 20) return;
    setPollOptions([...pollOptions, ""]);
  }

  function removePollOption(index: number) {
    if (pollOptions.length <= 2) return;
    setPollOptions(pollOptions.filter((_, i) => i !== index));
    if (pollMaxChoices > pollOptions.length - 1) {
      setPollMaxChoices(pollOptions.length - 1);
    }
  }

  function updatePollOption(index: number, value: string) {
    const updated = [...pollOptions];
    updated[index] = value;
    setPollOptions(updated);
  }

  const validOptionCount = pollOptions.filter((o) => o.trim()).length;

  return (
    <div className="space-y-6">
      <div className="flex items-center gap-3">
        <Link href="/forum">
          <Button variant="ghost" size="sm">
            <ArrowLeft className="mr-1 h-4 w-4" />
            Forum
          </Button>
        </Link>
      </div>

      <Card>
        <CardHeader>
          <CardTitle>New Topic</CardTitle>
        </CardHeader>
        <CardContent>
          <form onSubmit={handleSubmit} className="space-y-4">
            <div className="space-y-2">
              <Label htmlFor="board">Board *</Label>
              {isLoading ? (
                <Skeleton className="h-10 w-full" />
              ) : (
                <Select value={boardId ? { value: boardId, label: boards?.find((b: { id: string; name: string }) => b.id === boardId)?.name || "" } : null} onValueChange={(v) => { if (v && typeof v === "object" && "value" in v) setBoardId((v as { value: string }).value); }}>
                  <SelectTrigger className="w-full">
                    <SelectValue placeholder="Select a board" />
                  </SelectTrigger>
                  <SelectContent>
                    {boards?.map((b: { id: string; name: string }) => (
                      <SelectItem key={b.id} value={{ value: b.id, label: b.name }}>
                        {b.name}
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
              )}
            </div>

            <div className="space-y-2">
              <Label htmlFor="title">Title *</Label>
              <Input
                id="title"
                value={title}
                onChange={(e) => setTitle(e.target.value)}
                placeholder="What do you want to discuss?"
                maxLength={200}
                required
              />
            </div>

            <div className="space-y-2">
              <Label htmlFor="tags">Tags</Label>
              <Input
                id="tags"
                value={tags}
                onChange={(e) => setTags(e.target.value)}
                placeholder="Comma separated: rust, help, discussion"
                maxLength={200}
              />
            </div>

            <div className="space-y-2">
              <Label>Content *</Label>
              <RichTextEditor
                markdown={content}
                onChange={setContent}
                placeholder="Write your topic content here..."
              />
            </div>

            <div className="space-y-3 rounded-lg border p-4">
              <div className="flex items-center justify-between">
                <Label className="flex items-center gap-2 text-sm font-medium">
                  <BarChart3 className="h-4 w-4" />
                  Attach a Poll
                </Label>
                <Button
                  type="button"
                  variant="ghost"
                  size="sm"
                  className="text-xs"
                  onClick={() => setShowPoll(!showPoll)}
                >
                  {showPoll ? "Remove Poll" : "Add Poll"}
                </Button>
              </div>

              {showPoll && (
                <div className="space-y-3 pt-2">
                  <div className="space-y-1.5">
                    <Label className="text-xs">Question *</Label>
                    <Input
                      value={pollQuestion}
                      onChange={(e) => setPollQuestion(e.target.value)}
                      placeholder="What do you want to ask?"
                      maxLength={200}
                    />
                  </div>

                  <div className="space-y-1.5">
                    <Label className="text-xs">Options * (min 2, max 20)</Label>
                    <div className="space-y-1.5">
                      {pollOptions.map((opt, i) => (
                        <div key={i} className="flex items-center gap-1.5">
                          <Input
                            value={opt}
                            onChange={(e) => updatePollOption(i, e.target.value)}
                            placeholder={`Option ${i + 1}`}
                            maxLength={200}
                          />
                          {pollOptions.length > 2 && (
                            <Button
                              type="button"
                              variant="ghost"
                              size="sm"
                              className="h-8 w-8 shrink-0 p-0"
                              onClick={() => removePollOption(i)}
                            >
                              <X className="h-3.5 w-3.5" />
                            </Button>
                          )}
                        </div>
                      ))}
                    </div>
                    {pollOptions.length < 20 && (
                      <Button
                        type="button"
                        variant="outline"
                        size="sm"
                        className="gap-1 text-xs"
                        onClick={addPollOption}
                      >
                        <Plus className="h-3 w-3" />
                        Add Option
                      </Button>
                    )}
                  </div>

                  {validOptionCount > 2 && (
                    <div className="space-y-1.5">
                      <Label className="text-xs">Max choices per user</Label>
                      <Select
                        value={String(pollMaxChoices)}
                        onValueChange={(v) => setPollMaxChoices(Number(v))}
                      >
                        <SelectTrigger className="w-32">
                          <SelectValue />
                        </SelectTrigger>
                        <SelectContent>
                          {Array.from({ length: Math.min(validOptionCount, 10) }, (_, i) => (
                            <SelectItem key={i + 1} value={String(i + 1)}>
                              {i + 1}
                            </SelectItem>
                          ))}
                        </SelectContent>
                      </Select>
                    </div>
                  )}
                </div>
              )}
            </div>

            <div className="flex gap-2">
              <Button type="submit" disabled={createMut.isPending}>
                <Send className="mr-1.5 h-3.5 w-3.5" />
                {createMut.isPending ? "Creating..." : "Create Topic"}
              </Button>
              <Button
                type="button"
                variant="outline"
                onClick={() => router.back()}
              >
                Cancel
              </Button>
            </div>
          </form>
        </CardContent>
      </Card>
    </div>
  );
}
