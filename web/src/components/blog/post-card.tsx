"use client";

import Link from "next/link";
import Image from "next/image";
import { Eye } from "lucide-react";
import { Card, CardContent } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import type { Post } from "@/lib/api";

interface PostCardProps {
  post: Post;
}

function formatDate(dateStr: string): string {
  return new Date(dateStr).toLocaleDateString("en-US", {
    year: "numeric",
    month: "short",
    day: "numeric",
  });
}

export function PostCard({ post }: PostCardProps) {
  const displayTitle = post.title_highlight || post.title;
  const hasTitleHighlight = !!post.title_highlight;

  return (
    <Card className="group overflow-hidden transition-shadow hover:shadow-lg">
      {post.cover_image && (
        <Link href={`/posts/${post.slug}`}>
          <div className="aspect-video overflow-hidden relative">
            <Image
              src={post.cover_image}
              alt={post.title}
              fill
              className="object-cover transition-transform duration-300 group-hover:scale-105"
              sizes="(max-width: 640px) 100vw, (max-width: 1024px) 50vw, 33vw"
            />
          </div>
        </Link>
      )}
      <CardContent className="space-y-3 p-5">
        <div className="flex flex-wrap items-center gap-2">
          {post.category_name && (
            <Badge variant="secondary">{post.category_name}</Badge>
          )}
          {post.is_pinned && <Badge variant="default">Pinned</Badge>}
        </div>

        <Link href={`/posts/${post.slug}`}>
          {hasTitleHighlight ? (
            <h2
              className="text-xl font-semibold tracking-tight transition-colors group-hover:text-primary"
              dangerouslySetInnerHTML={{ __html: displayTitle }}
            />
          ) : (
            <h2 className="text-xl font-semibold tracking-tight transition-colors group-hover:text-primary">
              {post.title}
            </h2>
          )}
        </Link>

        {post.excerpt_highlight ? (
          <p
            className="line-clamp-2 text-sm text-muted-foreground"
            dangerouslySetInnerHTML={{ __html: post.excerpt_highlight }}
          />
        ) : post.excerpt ? (
          <p className="line-clamp-2 text-sm text-muted-foreground">
            {post.excerpt}
          </p>
        ) : null}

        <div className="flex flex-wrap items-center gap-2">
          {post.tags.map((tag) => (
            <Badge key={tag.id} variant="outline" className="text-xs">
              {tag.name}
            </Badge>
          ))}
        </div>

        <div className="flex items-center justify-between text-xs text-muted-foreground">
          <div className="flex items-center gap-3">
            <span>{post.author_name}</span>
            <span>
              {formatDate(post.published_at ?? post.created_at)}
            </span>
          </div>
          <div className="flex items-center gap-1">
            <Eye className="h-3.5 w-3.5" />
            <span>{post.view_count}</span>
          </div>
        </div>
      </CardContent>
    </Card>
  );
}
