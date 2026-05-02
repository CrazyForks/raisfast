"use client";

import React, { useState, useEffect } from "react";
import { useQuery, useMutation } from "@tanstack/react-query";
import { useRouter, useParams } from "next/navigation";
import Link from "next/link";
import { ArrowLeft, Send } from "lucide-react";
import { toast } from "sonner";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Skeleton } from "@/components/ui/skeleton";
import { RichTextEditor } from "@/components/editor/rich-text-editor";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { forum } from "@/lib/forum";
import { useAuthStore } from "@/stores/auth";

export default function EditTopicPage() {
  const router = useRouter();
  const params = useParams<{ id: string }>();
  const id = params.id;
  const { user } = useAuthStore();
  const isLoggedIn = useAuthStore((s) => s.isLoggedIn());
  const [mounted, setMounted] = useState(false);
  useEffect(() => setMounted(true), []);

  const [title, setTitle] = useState("");
  const [content, setContent] = useState("");

  const { data: topic, isLoading } = useQuery({
    queryKey: ["forum-topic", id],
    queryFn: () => forum.getTopic(id),
    enabled: !!id,
  });

  React.useEffect(() => {
    if (topic) {
      setTitle(topic.title || "");
      setContent(topic.content || "");
    }
  }, [topic]);

  const updateMut = useMutation({
    mutationFn: () =>
      forum.updateTopic(id, { title: title.trim(), content: content.trim() }),
    onSuccess: () => {
      toast.success("Topic updated");
      router.push(`/forum/topic/${id}`);
    },
    onError: () => toast.error("Failed to update topic"),
  });

  if (!mounted || !isLoggedIn) {
    return (
      <div className="py-16 text-center text-muted-foreground">
        <Link href="/auth/login" className="text-primary hover:underline">
          Login
        </Link>{" "}
        to edit a topic.
      </div>
    );
  }

  if (isLoading) {
    return (
      <div className="space-y-4">
        <Skeleton className="h-6 w-20" />
        <Skeleton className="h-10 w-full" />
        <Skeleton className="h-60 w-full" />
      </div>
    );
  }

  if (!topic) {
    return <p className="py-16 text-center text-muted-foreground">Topic not found</p>;
  }

  function handleSubmit(e: React.FormEvent) {
    e.preventDefault();
    if (!title.trim() || !content.trim()) {
      toast.error("Please fill in all required fields");
      return;
    }
    updateMut.mutate();
  }

  return (
    <div className="space-y-6">
      <div className="flex items-center gap-3">
        <Link href={`/forum/topic/${id}`}>
          <Button variant="ghost" size="sm">
            <ArrowLeft className="mr-1 h-4 w-4" />
            Back
          </Button>
        </Link>
      </div>

      <Card>
        <CardHeader>
          <CardTitle>Edit Topic</CardTitle>
        </CardHeader>
        <CardContent>
          <form onSubmit={handleSubmit} className="space-y-4">
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
              <Label>Content *</Label>
              <RichTextEditor
                markdown={content}
                onChange={setContent}
                placeholder="Write your topic content here..."
              />
            </div>

            <div className="flex gap-2">
              <Button type="submit" disabled={updateMut.isPending}>
                <Send className="mr-1.5 h-3.5 w-3.5" />
                {updateMut.isPending ? "Saving..." : "Save Changes"}
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
