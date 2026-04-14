"use client";

import { Suspense } from "react";
import { useQuery } from "@tanstack/react-query";
import { useSearchParams } from "next/navigation";
import { api, type PaginatedData, type Post } from "@/lib/api";
import { PostCard } from "@/components/blog/post-card";
import { SearchBar } from "@/components/blog/search-bar";
import { Pagination } from "@/components/common/pagination";
import { Skeleton } from "@/components/ui/skeleton";
import { Card, CardContent } from "@/components/ui/card";

function PostsContent() {
  const searchParams = useSearchParams();
  const page = Number(searchParams.get("page") ?? "1");
  const q = searchParams.get("q") ?? "";

  const { data, isLoading } = useQuery<PaginatedData<Post>>({
    queryKey: ["posts", page, q],
    queryFn: () =>
      api.get<PaginatedData<Post>>(
        `/posts?page=${page}&page_size=10${q ? `&q=${encodeURIComponent(q)}` : ""}`,
      ),
  });

  return (
    <div className="space-y-8">
      <SearchBar defaultValue={q} />

      {isLoading ? (
        <div className="grid gap-6 sm:grid-cols-2">
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
          <div className="grid gap-6 sm:grid-cols-2">
            {data.items.map((post) => (
              <PostCard key={post.id} post={post} />
            ))}
          </div>
          <Pagination page={data.page} pageSize={data.page_size} total={data.total} />
        </>
      ) : (
        <p className="py-16 text-center text-muted-foreground">No posts found</p>
      )}
    </div>
  );
}

export default function PostsPage() {
  return (
    <Suspense>
      <PostsContent />
    </Suspense>
  );
}
