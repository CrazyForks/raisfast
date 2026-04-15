"use client";

import { use, useState, useEffect, useMemo, useRef } from "react";
import Link from "next/link";
import { useRouter } from "next/navigation";
import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import { Plus, Trash2, ChevronUp, ChevronDown } from "lucide-react";
import { toast } from "sonner";

import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import { Skeleton } from "@/components/ui/skeleton";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { api, ApiError } from "@/lib/api";
import {
  type ContentTypeSchema,
  type PaginatedCmsResponse,
  getDisplayColumns,
  parseSort,
  getFieldLabel,
  getFieldByName,
  FieldCell,
} from "@/components/admin/field-renderer";

export default function ContentTypeListPage({
  params,
}: {
  params: Promise<{ singular: string }>;
}) {
  const { singular } = use(params);
  const router = useRouter();
  const queryClient = useQueryClient();
  const sortInitRef = useRef(false);

  const [page, setPage] = useState(1);
  const [statusFilter, setStatusFilter] = useState("");
  const pageSize = 50;
  const [sortField, setSortField] = useState("created_at");
  const [sortDirection, setSortDirection] = useState<"asc" | "desc">("desc");

  const schemaQuery = useQuery({
    queryKey: ["content-type", singular],
    queryFn: () =>
      api.get<ContentTypeSchema>(`/admin/content-types/${singular}`),
  });

  const schema = schemaQuery.data;

  useEffect(() => {
    if (schema?.list_view?.default_sort && !sortInitRef.current) {
      sortInitRef.current = true;
      const parsed = parseSort(schema.list_view.default_sort);
      setSortField(parsed.field);
      setSortDirection(parsed.direction);
    }
  }, [schema]);

  const listQuery = useQuery({
    queryKey: [
      "cms-items",
      schema?.plural,
      page,
      statusFilter,
    ],
    queryFn: () => {
      if (!schema)
        return {
          items: [],
          total: 0,
          page: 1,
          page_size: 50,
        } as PaginatedCmsResponse;
      const p = new URLSearchParams({
        page: String(page),
        page_size: String(pageSize),
      });
      if (statusFilter) p.set("status", statusFilter);
      return api.get<PaginatedCmsResponse>(
        `/admin/cms/${schema.plural}?${p.toString()}`,
      );
    },
    enabled: !!schema,
  });

  const sortedItems = useMemo(() => {
    const items = listQuery.data?.items ?? [];
    if (!sortField) return items;
    return [...items].sort((a, b) => {
      const aVal = a[sortField];
      const bVal = b[sortField];
      if (aVal == null && bVal == null) return 0;
      if (aVal == null) return 1;
      if (bVal == null) return -1;
      if (typeof aVal === "number" && typeof bVal === "number") {
        return sortDirection === "asc" ? aVal - bVal : bVal - aVal;
      }
      const cmp = String(aVal).localeCompare(String(bVal));
      return sortDirection === "asc" ? cmp : -cmp;
    });
  }, [listQuery.data?.items, sortField, sortDirection]);

  const deleteMutation = useMutation({
    mutationFn: ({
      plural,
      id,
    }: {
      plural: string;
      id: string;
    }) => api.delete(`/cms/${plural}/${id}`),
    onSuccess: () => {
      toast.success("Item deleted");
      if (schema) {
        queryClient.invalidateQueries({
          queryKey: ["cms-items", schema.plural],
        });
      }
    },
    onError: (err) => {
      if (err instanceof ApiError) {
        toast.error(err.message);
      } else {
        toast.error("Failed to delete item");
      }
    },
  });

  function handleSort(column: string) {
    if (column === sortField) {
      setSortDirection((prev) => (prev === "asc" ? "desc" : "asc"));
    } else {
      setSortField(column);
      setSortDirection("asc");
    }
  }

  function handleDelete(id: string) {
    if (!schema) return;
    if (confirm("Are you sure you want to delete this item?")) {
      deleteMutation.mutate({ plural: schema.plural, id });
    }
  }

  const totalPages = Math.ceil((listQuery.data?.total ?? 0) / pageSize);
  const columns = schema ? getDisplayColumns(schema) : [];

  if (schemaQuery.isLoading) {
    return (
      <div className="space-y-6">
        <Skeleton className="h-8 w-48" />
        <Skeleton className="h-64 w-full" />
      </div>
    );
  }

  if (schemaQuery.error || !schema) {
    return (
      <div className="space-y-6">
        <div className="flex items-center gap-4">
          <Link href="/admin/content-types">
            <Button variant="outline" size="sm">
              &larr; Back
            </Button>
          </Link>
          <h1 className="text-2xl font-bold">Content Type Not Found</h1>
        </div>
        <Card>
          <CardContent className="py-8 text-center text-muted-foreground">
            Content type &quot;{singular}&quot; not found.
          </CardContent>
        </Card>
      </div>
    );
  }

  return (
    <div className="space-y-6">
      <div className="flex items-center justify-between">
        <div className="flex items-center gap-4">
          <Link href="/admin/content-types">
            <Button variant="outline" size="sm">
              &larr; Back
            </Button>
          </Link>
          <h1 className="text-2xl font-bold">{schema.name}</h1>
        </div>
        <Link href={`/admin/content-types/${singular}/new`}>
          <Button>
            <Plus className="size-4" />
            New {schema.name}
          </Button>
        </Link>
      </div>

      {schema.draft_publish && (
        <div className="flex gap-1 bg-muted rounded-lg p-[3px] w-fit">
          {(["all", "draft", "published", "archived"] as const).map(
            (status) => {
              const isActive =
                status === "all"
                  ? statusFilter === ""
                  : statusFilter === status;
              return (
                <button
                  key={status}
                  type="button"
                  onClick={() => {
                    setStatusFilter(status === "all" ? "" : status);
                    setPage(1);
                  }}
                  className={`px-3 py-1 text-sm rounded-md transition-colors ${
                    isActive
                      ? "bg-background text-foreground shadow-sm"
                      : "text-muted-foreground hover:text-foreground"
                  }`}
                >
                  {status.charAt(0).toUpperCase() + status.slice(1)}
                </button>
              );
            },
          )}
        </div>
      )}

      <Card>
        <CardContent className="p-0">
          <Table>
            <TableHeader>
              <TableRow>
                {columns.map((col) => {
                  const field = getFieldByName(schema, col);
                  const label = field ? getFieldLabel(field) : col;
                  return (
                    <TableHead key={col}>
                      <button
                        type="button"
                        className="inline-flex items-center gap-1 hover:text-foreground transition-colors"
                        onClick={() => handleSort(col)}
                      >
                        {label}
                        {sortField === col ? (
                          sortDirection === "asc" ? (
                            <ChevronUp className="size-3" />
                          ) : (
                            <ChevronDown className="size-3" />
                          )
                        ) : null}
                      </button>
                    </TableHead>
                  );
                })}
                <TableHead className="text-right">Actions</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {listQuery.isLoading ? (
                <TableRow>
                  <TableCell
                    colSpan={columns.length + 1}
                    className="text-center py-8"
                  >
                    Loading...
                  </TableCell>
                </TableRow>
              ) : sortedItems.length === 0 ? (
                <TableRow>
                  <TableCell
                    colSpan={columns.length + 1}
                    className="text-center py-8"
                  >
                    No items found.
                  </TableCell>
                </TableRow>
              ) : (
                sortedItems.map((item) => (
                  <TableRow
                    key={item.id}
                    className="cursor-pointer"
                    onClick={() =>
                      router.push(
                        `/admin/content-types/${singular}/${item.id}/edit`,
                      )
                    }
                  >
                    {columns.map((col) => {
                      const field = getFieldByName(schema, col);
                      return (
                        <TableCell key={col}>
                          <FieldCell
                            field={field}
                            value={item[col]}
                            columnName={col}
                          />
                        </TableCell>
                      );
                    })}
                    <TableCell className="text-right">
                      <Button
                        variant="ghost"
                        size="icon-sm"
                        onClick={(e) => {
                          e.stopPropagation();
                          handleDelete(item.id);
                        }}
                        disabled={deleteMutation.isPending}
                      >
                        <Trash2 className="size-4" />
                      </Button>
                    </TableCell>
                  </TableRow>
                ))
              )}
            </TableBody>
          </Table>
        </CardContent>
      </Card>

      {totalPages > 1 && (
        <div className="flex items-center justify-center gap-2">
          <Button
            variant="outline"
            size="sm"
            disabled={page <= 1}
            onClick={() => setPage((p) => p - 1)}
          >
            Previous
          </Button>
          <span className="text-sm text-muted-foreground">
            Page {page} of {totalPages}
          </span>
          <Button
            variant="outline"
            size="sm"
            disabled={page >= totalPages}
            onClick={() => setPage((p) => p + 1)}
          >
            Next
          </Button>
        </div>
      )}
    </div>
  );
}
