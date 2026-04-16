"use client";

import { useState } from "react";
import { useRouter } from "next/navigation";
import { useForm } from "react-hook-form";
import { z } from "zod";
import { zodResolver } from "@hookform/resolvers/zod";
import { useQuery, useMutation } from "@tanstack/react-query";
import { toast } from "sonner";
import Link from "next/link";

import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Textarea } from "@/components/ui/textarea";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { api, ApiError } from "@/lib/api";

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
  tag_ids: z.string().optional(),
});

type PostForm = z.infer<typeof postSchema>;

export default function NewPostPage() {
  const router = useRouter();

  const categoriesQuery = useQuery({
    queryKey: ["categories"],
    queryFn: () => api.get<Category[]>("/categories"),
  });

  const tagsQuery = useQuery({
    queryKey: ["tags"],
    queryFn: () => api.get<Tag[]>("/tags"),
  });

  const [selectedTags, setSelectedTags] = useState<string[]>([]);

  const {
    register,
    handleSubmit,
    setValue,
    watch,
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
  });

  const statusValue = watch("status");
  const categoryValue = watch("category_id");

  function toggleTag(tagId: string) {
    setSelectedTags((prev) =>
      prev.includes(tagId)
        ? prev.filter((t) => t !== tagId)
        : [...prev, tagId],
    );
  }

  const createMutation = useMutation({
    mutationFn: (values: PostForm) =>
      api.post("/posts", {
        title: values.title,
        content: values.content,
        excerpt: values.excerpt || undefined,
        status: values.status,
        category_id: values.category_id || undefined,
        tag_ids: selectedTags.length > 0 ? selectedTags : undefined,
      }),
    onSuccess: () => {
      toast.success("Post created");
      router.push("/admin/posts");
    },
    onError: (err) => {
      if (err instanceof ApiError) {
        toast.error(err.message);
      } else {
        toast.error("Failed to create post");
      }
    },
  });

  const categories = categoriesQuery.data ?? [];
  const tags = tagsQuery.data ?? [];

  return (
    <div className="space-y-6">
      <div className="flex items-center gap-4">
        <Link href="/admin/posts">
          <Button variant="outline" size="sm">
            &larr; Back
          </Button>
        </Link>
        <h1 className="text-2xl font-bold">New Post</h1>
      </div>

      <Card>
        <CardContent className="pt-6">
          <form onSubmit={handleSubmit((v) => createMutation.mutate(v))} className="space-y-6">
            <div className="space-y-2">
              <Label htmlFor="title">Title</Label>
              <Input
                id="title"
                placeholder="Post title"
                {...register("title")}
              />
              {errors.title && (
                <p className="text-sm text-red-500">{errors.title.message}</p>
              )}
            </div>

            <div className="space-y-2">
              <Label htmlFor="content">Content</Label>
              <Textarea
                id="content"
                placeholder="Write your post content..."
                rows={15}
                {...register("content")}
              />
              {errors.content && (
                <p className="text-sm text-red-500">
                  {errors.content.message}
                </p>
              )}
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
                  onValueChange={(val) =>
                    val && setValue("status", val as "draft" | "published")
                  }
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
                  onValueChange={(val) =>
                    val && setValue("category_id", val)
                  }
                >
                  <SelectTrigger className="w-full">
                    <SelectValue placeholder="Select category" />
                  </SelectTrigger>
                  <SelectContent>
                    {categories.map((cat) => (
                      <SelectItem key={cat.id} value={cat.id}>
                        {cat.name}
                      </SelectItem>
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
              <Button type="submit" disabled={createMutation.isPending}>
                {createMutation.isPending ? "Creating..." : "Create Post"}
              </Button>
              <Link href="/admin/posts">
                <Button type="button" variant="outline">
                  Cancel
                </Button>
              </Link>
            </div>
          </form>
        </CardContent>
      </Card>
    </div>
  );
}
