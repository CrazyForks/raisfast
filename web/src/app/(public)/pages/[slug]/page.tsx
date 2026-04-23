"use client";

import { useQuery } from "@tanstack/react-query";
import { useParams } from "next/navigation";
import Link from "next/link";
import { ArrowLeft } from "lucide-react";
import { Skeleton } from "@/components/ui/skeleton";
import { page } from "@/lib/page";
import { BlockRenderer } from "@/components/public/block-renderer";
import { PostContent } from "@/components/blog/post-content";

export default function PublicPagePage() {
  const { slug } = useParams<{ slug: string }>();

  const { data: pg, isLoading } = useQuery({
    queryKey: ["public-page", slug],
    queryFn: () => page.getBySlug(slug),
  });

  if (isLoading) {
    return (
      <div className="space-y-6">
        <Skeleton className="h-8 w-3/4" />
        <Skeleton className="h-64" />
      </div>
    );
  }

  if (!pg) {
    return (
      <div className="py-16 text-center">
        <h1 className="text-2xl font-bold mb-2">Page Not Found</h1>
        <Link href="/" className="text-primary hover:underline">Go Home</Link>
      </div>
    );
  }

  let parsedBlocks: Record<string, unknown>[] = [];
  if (pg.blocks) {
    try {
      parsedBlocks = JSON.parse(pg.blocks);
    } catch { /* ignore */ }
  }

  return (
    <div>
      {parsedBlocks.length > 0 ? (
        <BlockRenderer blocks={parsedBlocks} />
      ) : pg.content ? (
        <article className="max-w-3xl mx-auto px-6 py-12">
          <Link
            href="/"
            className="inline-flex items-center gap-1 text-sm text-muted-foreground hover:text-foreground mb-8"
          >
            <ArrowLeft className="h-4 w-4" />
            Home
          </Link>
          <h1 className="text-4xl font-bold mb-8">{pg.title}</h1>
          <PostContent content={pg.content} />
        </article>
      ) : (
        <article className="max-w-3xl mx-auto px-6 py-12">
          <h1 className="text-4xl font-bold mb-8">{pg.title}</h1>
          <p className="text-muted-foreground">This page has no content yet.</p>
        </article>
      )}
    </div>
  );
}
