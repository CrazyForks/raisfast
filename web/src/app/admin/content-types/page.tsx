"use client";

import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import { useRouter } from "next/navigation";
import Link from "next/link";
import { Layers, Plus, Trash2, Pencil, Package } from "lucide-react";
import { toast } from "sonner";

import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { Card, CardContent, CardHeader, CardTitle, CardDescription } from "@/components/ui/card";
import { Skeleton } from "@/components/ui/skeleton";
import { api, ApiError } from "@/lib/api";
import type { ContentTypeSchema } from "@/components/admin/field-renderer";

export default function ContentTypesPage() {
  const router = useRouter();
  const queryClient = useQueryClient();

  const query = useQuery({
    queryKey: ["content-types"],
    queryFn: () => api.get<ContentTypeSchema[]>("/admin/content-types"),
  });

  const deleteMutation = useMutation({
    mutationFn: (singular: string) =>
      api.delete(`/admin/content-types/${singular}`),
    onSuccess: () => {
      toast.success("Content type deleted");
      queryClient.invalidateQueries({ queryKey: ["content-types"] });
    },
    onError: (err) => {
      if (err instanceof ApiError) {
        toast.error(err.message);
      } else {
        toast.error("Failed to delete");
      }
    },
  });

  function handleDelete(e: React.MouseEvent, singular: string) {
    e.preventDefault();
    e.stopPropagation();
    if (confirm(`Delete content type "${singular}"? This removes the schema file but NOT the database table.`)) {
      deleteMutation.mutate(singular);
    }
  }

  if (query.isLoading) {
    return (
      <div className="space-y-6">
        <div className="flex items-center justify-between">
          <h1 className="text-2xl font-bold">Content Types</h1>
        </div>
        <div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-3">
          {Array.from({ length: 3 }).map((_, i) => (
            <Skeleton key={i} className="h-32" />
          ))}
        </div>
      </div>
    );
  }

  const types = query.data ?? [];

  return (
    <div className="space-y-6">
      <div className="flex items-center justify-between">
        <h1 className="text-2xl font-bold">Content Types</h1>
        <div className="flex gap-2">
          <Link href="/admin/content-types/builder">
            <Button>
              <Plus className="size-4" />
              Create Content Type
            </Button>
          </Link>
        </div>
      </div>

      {types.length === 0 ? (
        <Card>
          <CardContent className="py-12 text-center space-y-4">
            <Layers className="size-12 mx-auto text-muted-foreground" />
            <div>
              <h3 className="text-lg font-medium">No content types yet</h3>
              <p className="text-sm text-muted-foreground mt-1">
                Create your first content type to start building your CMS.
              </p>
            </div>
            <Link href="/admin/content-types/builder">
              <Button>
                <Plus className="size-4" />
                Create Content Type
              </Button>
            </Link>
          </CardContent>
        </Card>
      ) : (
        <div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-3">
          {types.map((ct) => (
            <Link key={ct.singular} href={`/admin/content-types/${ct.singular}`}>
              <Card className="hover:bg-muted/50 transition-colors cursor-pointer h-full group">
                <CardHeader>
                  <div className="flex items-center justify-between">
                    <div className="flex items-center gap-2">
                      <Layers className="size-5 text-muted-foreground" />
                      <CardTitle>{ct.name}</CardTitle>
                      {ct.extension_id && (
                        <Badge
                          variant="secondary"
                          className="gap-1 text-xs cursor-pointer"
                          onClick={(e) => {
                            e.stopPropagation();
                            e.preventDefault();
                            router.push(`/admin/extensions/${ct.extension_id}`);
                          }}
                        >
                          <Package className="size-3" />
                          {ct.extension_id}
                        </Badge>
                      )}
                    </div>
                    <div className="flex gap-1 opacity-0 group-hover:opacity-100 transition-opacity">
                      <Button
                        variant="ghost"
                        size="icon-sm"
                        onClick={(e) => {
                          e.stopPropagation();
                          router.push(`/admin/content-types/builder?edit=${ct.singular}`);
                        }}
                      >
                        <Pencil className="size-4" />
                      </Button>
                      <Button
                        variant="ghost"
                        size="icon-sm"
                        onClick={(e) => handleDelete(e, ct.singular)}
                        disabled={deleteMutation.isPending}
                      >
                        <Trash2 className="size-4 text-destructive" />
                      </Button>
                    </div>
                  </div>
                  {ct.description && (
                    <CardDescription>{ct.description}</CardDescription>
                  )}
                </CardHeader>
                <CardContent>
                  <div className="flex items-center justify-between text-sm text-muted-foreground">
                    <span>{ct.fields.length} field{ct.fields.length !== 1 ? "s" : ""}</span>
                    <span className="font-mono text-xs">{ct.table}</span>
                  </div>
                </CardContent>
              </Card>
            </Link>
          ))}
        </div>
      )}
    </div>
  );
}
