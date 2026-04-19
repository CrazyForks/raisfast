"use client";

import { useCallback, useMemo, useState } from "react";
import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import {
  LayoutGrid,
  List,
  Search,
  ArrowUpDown,
} from "lucide-react";
import { toast } from "sonner";

import { useT } from "@/lib/i18n";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Skeleton } from "@/components/ui/skeleton";
import { api, ApiError, type MediaFile, type PaginatedData } from "@/lib/api";

import { MediaSidebar } from "@/components/admin/media/media-sidebar";
import { MediaUpload } from "@/components/admin/media/media-upload";
import { MediaGrid } from "@/components/admin/media/media-grid";
import { MediaList } from "@/components/admin/media/media-list";
import { MediaDetailPanel } from "@/components/admin/media/media-detail-panel";
import {
  matchesCategory,
  sortFiles,
  getAcceptForCategory,
  formatFileSize,
  type FileCategory,
} from "@/components/admin/media/media-utils";

type ViewMode = "grid" | "list";
type SortField = "created_at" | "filename" | "size";
type SortOrder = "asc" | "desc";

export default function MediaPage() {
  const { t } = useT();
  const queryClient = useQueryClient();
  const [page, setPage] = useState(1);
  const [viewMode, setViewMode] = useState<ViewMode>("grid");
  const [category, setCategory] = useState<FileCategory>("all");
  const [search, setSearch] = useState("");
  const [sortField, setSortField] = useState<SortField>("created_at");
  const [sortOrder, setSortOrder] = useState<SortOrder>("desc");
  const [selectedFile, setSelectedFile] = useState<MediaFile | null>(null);

  const pageSize = 20;

  const mediaQuery = useQuery({
    queryKey: ["media", page],
    queryFn: () =>
      api.get<PaginatedData<MediaFile>>(
        `/media?page=${page}&page_size=${pageSize}`,
      ),
  });

  const statsQuery = useQuery({
    queryKey: ["media-stats"],
    queryFn: () =>
      api.get<{
        total_files: number;
        total_size: number;
        by_type: { mimetype: string; count: number; total_size: number }[];
      }>("/media/stats"),
  });

  const deleteMutation = useMutation({
    mutationFn: (id: string) => api.delete(`/media/${id}`),
    onSuccess: () => {
      toast.success(t("media.fileDeleted"));
      setSelectedFile(null);
      queryClient.invalidateQueries({ queryKey: ["media"] });
    },
    onError: (err) => {
      if (err instanceof ApiError) toast.error(err.message);
      else toast.error(t("media.failedToDelete"));
    },
  });

  function handleDelete(id: string) {
    if (confirm(t("media.confirmDelete"))) {
      deleteMutation.mutate(id);
    }
  }

  const allFiles = mediaQuery.data?.items ?? [];

  const filteredFiles = useMemo(() => {
    let files = allFiles;
    if (category !== "all") {
      files = files.filter((f) => matchesCategory(f, category));
    }
    if (search.trim()) {
      const q = search.toLowerCase();
      files = files.filter((f) => f.filename.toLowerCase().includes(q));
    }
    return sortFiles(files, sortField, sortOrder);
  }, [allFiles, category, search, sortField, sortOrder]);

  const totalPages = Math.ceil((mediaQuery.data?.total ?? 0) / pageSize);

  const toggleSort = useCallback(
    (field: SortField) => {
      if (sortField === field) {
        setSortOrder((o) => (o === "asc" ? "desc" : "asc"));
      } else {
        setSortField(field);
        setSortOrder("desc");
      }
    },
    [sortField],
  );

  return (
    <div className="flex h-[calc(100vh-8rem)] gap-0">
      {/* Sidebar */}
      <aside className="w-48 shrink-0 border-r py-4 px-2 overflow-hidden flex flex-col gap-4">
        <h1 className="text-xl font-bold">{t("media.title")}</h1>
        {statsQuery.data && (
          <p className="text-xs text-muted-foreground">
            {t("media.files", { count: statsQuery.data.total_files })} &middot; {formatFileSize(statsQuery.data.total_size)}
          </p>
        )}
        <MediaSidebar
          files={allFiles}
          selected={category}
          onSelect={setCategory}
        />
      </aside>

      {/* Main content */}
      <div className="flex flex-1 min-w-0">
        <div className="flex-1 min-w-0 p-4 space-y-4 overflow-y-auto">
          {/* Toolbar */}
          <div className="flex items-center gap-2 flex-wrap">
            <div className="relative flex-1 max-w-xs">
              <Search className="absolute left-2.5 top-1/2 -translate-y-1/2 size-4 text-muted-foreground" />
              <Input
                placeholder={t("media.searchFiles")}
                className="pl-8 h-8"
                value={search}
                onChange={(e) => setSearch(e.target.value)}
              />
            </div>

            <Select
              value={sortField}
              onValueChange={(v) => toggleSort(v as SortField)}
            >
              <SelectTrigger className="w-32 h-8">
                <ArrowUpDown className="size-3.5 mr-1" />
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="created_at">{t("media.date")}</SelectItem>
                <SelectItem value="filename">{t("media.name")}</SelectItem>
                <SelectItem value="size">{t("media.size")}</SelectItem>
              </SelectContent>
            </Select>

            <Button
              variant="ghost"
              size="icon-xs"
              onClick={() =>
                setSortOrder((o) => (o === "asc" ? "desc" : "asc"))
              }
              title={sortOrder === "asc" ? t("media.ascending") : t("media.descending")}
            >
              {sortOrder === "asc" ? "↑" : "↓"}
            </Button>

            <div className="flex border rounded-md overflow-hidden">
              <Button
                variant={viewMode === "grid" ? "secondary" : "ghost"}
                size="icon-xs"
                onClick={() => setViewMode("grid")}
              >
                <LayoutGrid className="size-4" />
              </Button>
              <Button
                variant={viewMode === "list" ? "secondary" : "ghost"}
                size="icon-xs"
                onClick={() => setViewMode("list")}
              >
                <List className="size-4" />
              </Button>
            </div>
          </div>

          {/* Upload area */}
          <MediaUpload accept={getAcceptForCategory(category)} />

          {/* Content */}
          {mediaQuery.isLoading ? (
            viewMode === "grid" ? (
              <div className="grid gap-4 grid-cols-2 sm:grid-cols-3 lg:grid-cols-4 xl:grid-cols-5">
                {Array.from({ length: 10 }).map((_, i) => (
                  <Skeleton key={i} className="aspect-square rounded-xl" />
                ))}
              </div>
            ) : (
              <div className="space-y-2">
                {Array.from({ length: 8 }).map((_, i) => (
                  <Skeleton key={i} className="h-14 rounded" />
                ))}
              </div>
            )
          ) : filteredFiles.length === 0 ? null : viewMode === "grid" ? (
            <MediaGrid
              files={filteredFiles}
              onDelete={handleDelete}
              onSelect={setSelectedFile}
              selectedId={selectedFile?.id}
            />
          ) : (
            <MediaList
              files={filteredFiles}
              onDelete={handleDelete}
              onSelect={setSelectedFile}
              selectedId={selectedFile?.id}
            />
          )}

          {/* Pagination */}
          {totalPages > 1 && (
            <div className="flex items-center justify-center gap-2 pt-2">
              <Button
                variant="outline"
                size="sm"
                disabled={page <= 1}
                onClick={() => setPage((p) => p - 1)}
              >
                {t("common.previous")}
              </Button>
              <span className="text-sm text-muted-foreground">
                {t("common.pageOf", { page, total: totalPages })}
              </span>
              <Button
                variant="outline"
                size="sm"
                disabled={page >= totalPages}
                onClick={() => setPage((p) => p + 1)}
              >
                {t("common.next")}
              </Button>
            </div>
          )}
        </div>

        {/* Detail panel */}
        {selectedFile && (
          <MediaDetailPanel
            file={selectedFile}
            onDelete={handleDelete}
            onClose={() => setSelectedFile(null)}
          />
        )}
      </div>
    </div>
  );
}
