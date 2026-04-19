"use client";

import { useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { X, Check } from "lucide-react";

import { Input } from "@/components/ui/input";
import { Button } from "@/components/ui/button";
import { Skeleton } from "@/components/ui/skeleton";
import { api, type MediaFile, type PaginatedData } from "@/lib/api";
import {
  isImageMime,
  formatFileSize,
  getCategoryIcon,
  matchesCategory,
  type FileCategory,
} from "@/components/admin/media/media-utils";

interface MediaSelectorProps {
  onSelect: (file: MediaFile) => void;
  onClose: () => void;
  category?: FileCategory;
}

export function MediaSelector({ onSelect, onClose, category = "image" }: MediaSelectorProps) {
  const [search, setSearch] = useState("");
  const [page, setPage] = useState(1);
  const pageSize = 20;

  const query = useQuery({
    queryKey: ["media", page],
    queryFn: () =>
      api.get<PaginatedData<MediaFile>>(
        `/media?page=${page}&page_size=${pageSize}`,
      ),
  });

  const allFiles = query.data?.items ?? [];
  const filtered = allFiles
    .filter((f) => matchesCategory(f, category))
    .filter((f) =>
      search.trim()
        ? f.filename.toLowerCase().includes(search.toLowerCase())
        : true,
    );
  const totalPages = Math.ceil((query.data?.total ?? 0) / pageSize);

  return (
    <div className="space-y-3">
      <div className="flex items-center justify-between">
        <h3 className="text-sm font-medium">Select from Media</h3>
        <button
          onClick={onClose}
          className="text-muted-foreground hover:text-foreground"
        >
          <X className="size-4" />
        </button>
      </div>

      <Input
        placeholder="Search files..."
        value={search}
        onChange={(e) => setSearch(e.target.value)}
        className="h-8 text-sm"
      />

      {query.isLoading ? (
        <div className="grid grid-cols-4 gap-2">
          {Array.from({ length: 8 }).map((_, i) => (
            <Skeleton key={i} className="aspect-square rounded" />
          ))}
        </div>
      ) : filtered.length === 0 ? (
        <p className="py-8 text-center text-sm text-muted-foreground">
          No matching files found.
        </p>
      ) : (
        <div className="grid grid-cols-4 gap-2 max-h-64 overflow-y-auto">
          {filtered.map((file) => {
            const FileIcon = getCategoryIcon(file.mimetype);
            return (
              <button
                key={file.id}
                type="button"
                onClick={() => onSelect(file)}
                className="group relative aspect-square rounded border bg-muted overflow-hidden hover:ring-2 hover:ring-primary transition-all"
              >
                {isImageMime(file.mimetype) ? (
                  /* eslint-disable-next-line @next/next/no-img-element */
                  <img
                    src={file.url}
                    alt={file.filename}
                    className="object-cover w-full h-full"
                  />
                ) : (
                  <div className="flex flex-col items-center justify-center h-full gap-1 p-1">
                    <FileIcon className="size-6 text-muted-foreground" />
                    <span className="text-[9px] text-muted-foreground truncate w-full text-center">
                      {file.filename}
                    </span>
                  </div>
                )}
              <div className="absolute inset-x-0 bottom-0 bg-black/60 px-1.5 py-0.5 opacity-0 group-hover:opacity-100 transition-opacity">
                <p className="text-[10px] text-white truncate">{file.filename}</p>
                <p className="text-[9px] text-white/70">{formatFileSize(file.size)}</p>
              </div>
              <div className="absolute inset-0 flex items-center justify-center opacity-0 group-hover:opacity-100 transition-opacity">
                <div className="rounded-full bg-primary p-1">
                  <Check className="size-3 text-primary-foreground" />
                </div>
              </div>
            </button>
            );
          })}
        </div>
      )}

      {totalPages > 1 && (
        <div className="flex items-center justify-center gap-2">
          <Button
            variant="outline"
            size="xs"
            disabled={page <= 1}
            onClick={() => setPage((p) => p - 1)}
          >
            Prev
          </Button>
          <span className="text-xs text-muted-foreground">
            {page}/{totalPages}
          </span>
          <Button
            variant="outline"
            size="xs"
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
