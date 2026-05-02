"use client";

import Link from "next/link";
import { useRouter } from "next/navigation";
import { useState } from "react";
import { useQuery } from "@tanstack/react-query";
import type { Post } from "@/lib/types";
import { client } from "@/lib/raisfast";
import type { PaginatedData } from "@raisfast/sdk";
import { PostCard } from "@/components/blog/post-card";
import { Skeleton } from "@/components/ui/skeleton";
import { Card, CardContent } from "@/components/ui/card";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Search } from "lucide-react";

export default function HomePage() {
  const router = useRouter();
  const [query, setQuery] = useState("");

  const { data, isLoading } = useQuery<PaginatedData<Post>>({
    queryKey: ["posts", 1],
    queryFn: () => client.send<PaginatedData<Post>>("/posts?page=1&page_size=6"),
  });

  function handleSearch(e: React.FormEvent) {
    e.preventDefault();
    if (query.trim()) {
      router.push(`/posts?q=${encodeURIComponent(query.trim())}`);
    }
  }

  return (
    <div className="flex flex-1 flex-col">
      <section className="border-b py-20 text-center">
        <div className="mx-auto max-w-2xl space-y-6">
          <h1 className="text-4xl font-bold tracking-tight sm:text-5xl">Blog</h1>
          <p className="text-lg text-muted-foreground">
            Thoughts, stories, and ideas worth sharing.
          </p>
          <form onSubmit={handleSearch} className="mx-auto flex max-w-md gap-2">
            <div className="relative flex-1">
              <Search className="absolute left-3 top-1/2 h-4 w-4 -translate-y-1/2 text-muted-foreground" />
              <Input
                type="text"
                value={query}
                onChange={(e) => setQuery(e.target.value)}
                placeholder="Search posts..."
                className="pl-9"
              />
            </div>
            <Button type="submit" size="default">
              Search
            </Button>
          </form>
        </div>
      </section>

      <section className="py-12">
        <div className="mx-auto max-w-5xl px-4">
          {isLoading ? (
            <div className="grid gap-6 sm:grid-cols-2 lg:grid-cols-3">
              {Array.from({ length: 6 }).map((_, i) => (
                <Card key={i}>
                  <CardContent className="space-y-3 p-5">
                    <Skeleton className="h-4 w-20" />
                    <Skeleton className="h-6 w-3/4" />
                    <Skeleton className="h-4 w-full" />
                    <Skeleton className="h-4 w-2/3" />
                  </CardContent>
                </Card>
              ))}
            </div>
          ) : data && data.items.length > 0 ? (
            <>
              <div className="grid gap-6 sm:grid-cols-2 lg:grid-cols-3">
                {data.items.map((post) => (
                  <PostCard key={post.id} post={post} />
                ))}
              </div>
              <div className="mt-10 text-center">
                <Link href="/posts">
                  <Button variant="outline" size="lg">
                    View all posts
                  </Button>
                </Link>
              </div>
            </>
          ) : (
            <p className="py-16 text-center text-muted-foreground">
              No posts yet.
            </p>
          )}
        </div>
      </section>
    </div>
  );
}
