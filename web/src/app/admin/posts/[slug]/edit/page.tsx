"use client";

import { useState } from "react";
import { useRouter, useParams } from "next/navigation";
import { useForm } from "react-hook-form";
import { z } from "zod";
import { zodResolver } from "@hookform/resolvers/zod";
import { useQuery } from "@tanstack/react-query";
import { toast } from "sonner";
import Link from "next/link";

import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Textarea } from "@/components/ui/textarea";
import { Skeleton } from "@/components/ui/skeleton";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { api, ApiError, type Post } from "@/lib/api";

interface Category {
  id: string;
  name: string;
  slug: string;
}

interface Tag {
  id: string;
  name: string;
  slug: string;
}

const postSchema = z.object({
  title: z.string().min(1, "Title is required").max(200, "Title must be 200 characters or less"),
  content: z.string().min(1, "Content is required"),
  excerpt: z.string().optional(),
  status: z.enum(["draft", "published"]),
  category_id: z.string().optional(),
});

type PostForm = z.infer<typeof postSchema>;

export default function EditPostPage() {
  const router = useRouter();
  const params = useParams();
  const slug = params.slug as string;
  const [loading, setLoading] = useState(false);

  const postQuery = useQuery({
    queryKey: ["post", slug],
    queryFn: () => api.get<Post>(`/admin/posts/${slug}`),
  });

  const categoriesQuery = useQuery({
    queryKey: ["categories"],
    queryFn: () => api.get<Category[]>("/categories"),
  });

  const tagsQuery = useQuery({
    queryKey: ["tags"],
    queryFn: () => api.get<Tag[]>("/tags"),
  });

  const post = postQuery.data;

  const defaultTagIds = post?.tags?.map((t) => t.id) ?? [];
  const [selectedTags, setSelectedTags] = useState<string[]>([]);

  const {
    register,
    handleSubmit,
    setValue,
    watch,
    reset,
    formState: { errors },
  } = useForm<PostForm>({
    resolver: zodResolver(postSchema as never),
    defaultValues: {
      title: "",
      content: "",
      excerpt: "",
      status: "draft",
      category_id: "",
    },
    values: post
      ? {
          title: post.title,
          content: post.content,
          excerpt: post.excerpt ?? "",
          status: post.status as "draft" | "published",
          category_id: post.category_id ?? "",
        }
      : undefined,
  });

  if (post && selectedTags.length === 0 && defaultTagIds.length > 0) {
    setSelectedTags(defaultTagIds);
  }

  const statusValue = watch("status");
  const categoryValue = watch("category_id");

  function toggleTag(tagId: string) {
    setSelectedTags((prev) =>
      prev.includes(tagId) ? prev.filter((t) => t !== tagId) : [...prev, tagId],
    );
  }

  async function onSubmit(values: PostForm) {
    setLoading(true);
    try {
      await api.put(`/posts/${slug}`, {
        title: values.title,
        content: values.content,
        excerpt: values.excerpt || undefined,
        status: values.status,
        category_id: values.category_id || undefined,
        tag_ids: selectedTags.length > 0 ? selectedTags : undefined,
      });
      toast.success("Post updated");
      router.push("/admin/posts");
    } catch (err) {
      if (err instanceof ApiError) {
        toast.error(err.message);
      } else {
        toast.error("Failed to update post");
      }
    } finally {
      setLoading(false);
    }
  }

  const categories = categoriesQuery.data ?? [];
  const tags = tagsQuery.data ?? [];

  if (postQuery.isLoading) {
    return (
      <div className="space-y-6">
        <div className="flex items-center gap-4">
          <Link href="/admin/posts">
            <Button variant="outline" size="sm">&larr; Back</Button>
          </Link>
          <Skeleton className="h-8 w-32" />
        </div>
        <Card>
          <CardContent className="pt-6 space-y-6">
            <Skeleton className="h-10 w-full" />
            <Skeleton className="h-40 w-full" />
            <Skeleton className="h-10 w-full" />
          </CardContent>
        </Card>
      </div>
    );
  }

  if (postQuery.error) {
    return (
      <div className="space-y-6">
        <div className="flex items-center gap-4">
          <Link href="/admin/posts">
            <Button variant="outline" size="sm">&larr; Back</Button>
          </Link>
          <h1 className="text-2xl font-bold">Edit Post</h1>
        </div>
        <Card>
          <CardContent className="pt-6">
            <p className="text-muted-foreground">Failed to load post.</p>
          </CardContent>
        </Card>
      </div>
    );
  }

  return (
    <div className="space-y-6">
      <div className="flex items-center gap-4">
        <Link href="/admin/posts">
          <Button variant="outline" size="sm">&larr; Back</Button>
        </Link>
        <h1 className="text-2xl font-bold">Edit Post</h1>
      </div>

      <Card>
        <CardContent className="pt-6">
          <form onSubmit={handleSubmit(onSubmit)} className="space-y-6">
            <div className="space-y-2">
              <Label htmlFor="title">Title</Label>
              <Input id="title" placeholder="Post title" {...register("title")} />
              {errors.title && <p className="text-sm text-red-500">{errors.title.message}</p>}
            </div>

            <div className="space-y-2">
              <Label htmlFor="content">Content</Label>
              <Textarea
                id="content"
                placeholder="Write your post content..."
                rows={15}
                {...register("content")}
              />
              {errors.content && <p className="text-sm text-red-500">{errors.content.message}</p>}
            </div>

            <div className="space-y-2">
              <Label htmlFor="excerpt">Excerpt</Label>
              <Textarea
                id="excerpt"
                placeholder="Brief summary (optional)"
                rows={3}
                {...register("excerpt")}
              />
            </div>

            <div className="grid gap-6 sm:grid-cols-2">
              <div className="space-y-2">
                <Label>Status</Label>
                <Select
                  value={statusValue}
                  onValueChange={(val) => val && setValue("status", val as "draft" | "published")}
                >
                  <SelectTrigger className="w-full">
                    <SelectValue placeholder="Select status" />
                  </SelectTrigger>
                  <SelectContent>
                    <SelectItem value="draft">Draft</SelectItem>
                    <SelectItem value="published">Published</SelectItem>
                  </SelectContent>
                </Select>
              </div>

              <div className="space-y-2">
                <Label>Category</Label>
                <Select
                  value={categoryValue}
                  onValueChange={(val) => val && setValue("category_id", val)}
                >
                  <SelectTrigger className="w-full">
                    <SelectValue placeholder="Select category" />
                  </SelectTrigger>
                  <SelectContent>
                    {categories.map((cat) => (
                      <SelectItem key={cat.id} value={cat.id}>{cat.name}</SelectItem>
                    ))}
                  </SelectContent>
                </Select>
              </div>
            </div>

            <div className="space-y-2">
              <Label>Tags</Label>
              {tags.length === 0 ? (
                <p className="text-sm text-muted-foreground">No tags available</p>
              ) : (
                <div className="flex flex-wrap gap-2">
                  {tags.map((tag) => (
                    <button
                      key={tag.id}
                      type="button"
                      onClick={() => toggleTag(tag.id)}
                      className={`rounded-full border px-3 py-1 text-sm transition-colors ${
                        selectedTags.includes(tag.id)
                          ? "bg-primary text-primary-foreground border-primary"
                          : "bg-background text-foreground border-border hover:bg-muted"
                      }`}
                    >
                      {tag.name}
                    </button>
                  ))}
                </div>
              )}
            </div>

            <div className="flex gap-2">
              <Button type="submit" disabled={loading}>
                {loading ? "Saving..." : "Save Changes"}
              </Button>
              <Link href="/admin/posts">
                <Button type="button" variant="outline">Cancel</Button>
              </Link>
            </div>
          </form>
        </CardContent>
      </Card>
    </div>
  );
}
