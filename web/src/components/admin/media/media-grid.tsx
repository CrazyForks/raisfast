"use client";

import type { MediaFile } from "@/lib/api";
import { MoreVertical, Trash2, Copy, Download } from "lucide-react";
import { toast } from "sonner";

import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { Button } from "@/components/ui/button";
import { Card } from "@/components/ui/card";
import {
  formatFileSize,
  getCategoryIcon,
  isImageMime,
} from "./media-utils";

interface MediaGridProps {
  files: MediaFile[];
  onDelete: (id: string) => void;
  onSelect?: (file: MediaFile) => void;
  selectedId?: string | null;
}

export function MediaGrid({
  files,
  onDelete,
  onSelect,
  selectedId,
}: MediaGridProps) {
  return (
    <div className="grid gap-4 grid-cols-2 sm:grid-cols-3 lg:grid-cols-4 xl:grid-cols-5">
      {files.map((file) => {
        const Icon = getCategoryIcon(file.mimetype);
        const isSelected = selectedId === file.id;

        return (
          <Card
            key={file.id}
            className={`group overflow-hidden cursor-pointer transition-shadow hover:shadow-md ${
              isSelected ? "ring-2 ring-primary" : ""
            }`}
            onClick={() => onSelect?.(file)}
          >
            <div className="aspect-square bg-muted flex items-center justify-center overflow-hidden relative">
              {isImageMime(file.mimetype) ? (
                /* eslint-disable-next-line @next/next/no-img-element */
                <img
                  src={file.url}
                  alt={file.filename}
                  className="object-cover w-full h-full"
                />
              ) : (
                <Icon className="size-12 text-muted-foreground" />
              )}
            </div>
            <div className="p-3 space-y-1">
              <p className="text-sm font-medium truncate" title={file.filename}>
                {file.filename}
              </p>
              <p className="text-xs text-muted-foreground">
                {formatFileSize(file.size)}
              </p>
              <div className="flex items-center justify-between pt-1">
                <p className="text-xs text-muted-foreground">
                  {new Date(file.created_at).toLocaleDateString()}
                </p>
                <DropdownMenu>
                  <DropdownMenuTrigger
                    render={
                      <Button
                        variant="ghost"
                        size="icon-xs"
                        onClick={(e: React.MouseEvent) => e.stopPropagation()}
                      >
                        <MoreVertical className="size-3.5" />
                      </Button>
                    }
                  />
                  <DropdownMenuContent align="end">
                    <DropdownMenuItem
                      onSelect={() => {
                        navigator.clipboard.writeText(file.url);
                        toast.success("URL copied");
                      }}
                    >
                      <Copy className="size-4" />
                      Copy URL
                    </DropdownMenuItem>
                    <DropdownMenuItem
                      onSelect={() => window.open(file.url, "_blank")}
                    >
                      <Download className="size-4" />
                      Download
                    </DropdownMenuItem>
                    <DropdownMenuItem
                      className="text-destructive"
                      onSelect={(e) => {
                        e.preventDefault();
                        onDelete(file.id);
                      }}
                    >
                      <Trash2 className="size-4" />
                      Delete
                    </DropdownMenuItem>
                  </DropdownMenuContent>
                </DropdownMenu>
              </div>
            </div>
          </Card>
        );
      })}
    </div>
  );
}
