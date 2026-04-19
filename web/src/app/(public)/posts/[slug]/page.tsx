"use client";

import { useQuery } from "@tanstack/react-query";
import { useParams } from "next/navigation";
import Link from "next/link";
import { ArrowLeft, Eye } from "lucide-react";
import { api, type Post } from "@/lib/api";
import { PostContent } from "@/components/blog/post-content";
import { CommentSection } from "@/components/blog/comment-section";
import { Badge } from "@/components/ui/badge";
import { Separator } from "@/components/ui/separator";
import { Skeleton } from "@/components/ui/skeleton";

function formatDate(dateStr: string): string {
  return new Date(dateStr).toLocaleDateString("en-US", {
    year: "numeric",
    month: "long",
    day: "numeric",
  });
}

export default function PostDetailPage() {
  const { slug } = useParams<{ slug: string }>();

  const { data: post, isLoading } = useQuery<Post>({
    queryKey: ["post", slug],
    queryFn: () => api.get<Post>(`/posts/${slug}`),
  });

  if (isLoading) {
    return (
      <div className="space-y-6">
        <Skeleton className="h-5 w-24" />
        <Skeleton className="h-10 w-3/4" />
        <Skeleton className="h-4 w-48" />
        <div className="space-y-4">
          <Skeleton className="h-4 w-full" />
          <Skeleton className="h-4 w-full" />
          <Skeleton className="h-4 w-2/3" />
        </div>
      </div>
    );
  }

  if (!post) {
    return (
      <div className="py-16 text-center text-muted-foreground">Post not found</div>
    );
  }

  return (
    <article className="space-y-8">
      <Link
        href="/posts"
        className="inline-flex items-center gap-1 text-sm text-muted-foreground hover:text-foreground"
      >
        <ArrowLeft className="h-4 w-4" />
        Back to posts
      </Link>

      <div className="space-y-4">
        <h1 className="text-3xl font-bold tracking-tight sm:text-4xl">
          {post.title}
        </h1>

        <div className="flex flex-wrap items-center gap-3 text-sm text-muted-foreground">
          <span>{post.author_name}</span>
          <span>&middot;</span>
          <span>{formatDate(post.published_at ?? post.created_at)}</span>
          {post.category_name && (
            <>
              <span>&middot;</span>
              <Badge variant="secondary">{post.category_name}</Badge>
            </>
          )}
          {post.tags.map((tag) => (
            <Badge key={tag.id} variant="outline">
              {tag.name}
            </Badge>
          ))}
          <span className="inline-flex items-center gap-1">
            <Eye className="h-3.5 w-3.5" />
            {post.view_count}
          </span>
        </div>
      </div>

      <PostContent content={post.content} />

      <Separator />

      <CommentSection postSlug={slug} />
    </article>
  );
}
