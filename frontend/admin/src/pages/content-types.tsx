
import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import { useRouter } from "@/lib/navigation";
import Link from "@/lib/link";
import { Layers, Plus, Trash2, Pencil, MoreVertical } from "lucide-react";
import { toast } from "sonner";

import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { Card, CardContent, CardHeader, CardTitle, CardDescription } from "@/components/ui/card";
import { Skeleton } from "@/components/ui/skeleton";
import { DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuTrigger } from "@/components/ui/dropdown-menu";
import { client } from "@/lib/raisfast";
import { SDKError } from "@raisfast/sdk";
import { useT } from "@/lib/i18n";
import type { ContentTypeSchema } from "@/components/admin/field-renderer";

export default function ContentTypesPage() {
  const router = useRouter();
  const { t } = useT();
  const queryClient = useQueryClient();

  const query = useQuery({
    queryKey: ["content-types"],
    queryFn: () => client.send<ContentTypeSchema[]>("/admin/content-types"),
  });

  const deleteMutation = useMutation({
    mutationFn: (singular: string) =>
      client.admin.contentTypes.delete(singular),
    onSuccess: () => {
      toast.success(t("contentTypes.contentTypeDeleted"));
      queryClient.invalidateQueries({ queryKey: ["content-types"] });
    },
    onError: (err) => {
      if (err instanceof SDKError) {
        toast.error(err.message);
      } else {
        toast.error(t("contentTypes.failedToDelete"));
      }
    },
  });

  function handleDelete(e: React.MouseEvent, singular: string) {
    e.preventDefault();
    e.stopPropagation();
    if (confirm(t("contentTypes.confirmDelete", { singular }))) {
      deleteMutation.mutate(singular);
    }
  }

  if (query.isLoading) {
    return (
      <div className="space-y-6">
        <div className="flex items-center justify-between">
          <h1 className="text-2xl font-bold">{t("contentTypes.title")}</h1>
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
        <h1 className="text-2xl font-bold">{t("contentTypes.title")}</h1>
        <div className="flex gap-2">
          <Link href="/content-types/builder">
            <Button>
              <Plus className="size-4" />
              {t("contentTypes.createContentType")}
            </Button>
          </Link>
        </div>
      </div>

      {types.length === 0 ? (
        <Card>
          <CardContent className="py-12 text-center space-y-4">
            <Layers className="size-12 mx-auto text-muted-foreground" />
            <div>
              <h3 className="text-lg font-medium">{t("contentTypes.noContentTypes")}</h3>
              <p className="text-sm text-muted-foreground mt-1">
                {t("contentTypes.noContentTypesDesc")}
              </p>
            </div>
            <Link href="/content-types/builder">
              <Button>
                <Plus className="size-4" />
                {t("contentTypes.createContentType")}
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
                    </div>
                    <DropdownMenu>
                      <DropdownMenuTrigger
                        className="inline-flex items-center justify-center rounded-md p-1 hover:bg-muted transition-colors"
                        onClick={(e) => { e.stopPropagation(); e.preventDefault(); }}
                      >
                        <MoreVertical className="size-4" />
                      </DropdownMenuTrigger>
                      <DropdownMenuContent align="end">
                        <DropdownMenuItem
                          onClick={(e) => {
                            e.stopPropagation();
                            router.push(`/admin/content-types/builder?edit=${ct.singular}`);
                          }}
                        >
                          <Pencil className="size-4 mr-2" />
                          {t("common.edit")}
                        </DropdownMenuItem>
                        <DropdownMenuItem
                          className="text-destructive"
                          onClick={(e) => handleDelete(e, ct.singular)}
                          disabled={deleteMutation.isPending}
                        >
                          <Trash2 className="size-4 mr-2" />
                          {t("common.delete")}
                        </DropdownMenuItem>
                      </DropdownMenuContent>
                    </DropdownMenu>
                  </div>
                  {ct.description && (
                    <CardDescription>{ct.description}</CardDescription>
                  )}
                </CardHeader>
                <CardContent>
                  <div className="flex items-center justify-between text-sm text-muted-foreground">
                    <span>{t("contentTypes.fields", { count: ct.fields.length })}</span>
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
