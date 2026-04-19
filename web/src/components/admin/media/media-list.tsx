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
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import {
  formatFileSize,
  getCategoryIcon,
  isImageMime,
} from "./media-utils";

interface MediaListProps {
  files: MediaFile[];
  onDelete: (id: string) => void;
  onSelect?: (file: MediaFile) => void;
  selectedId?: string | null;
}

export function MediaList({
  files,
  onDelete,
  onSelect,
  selectedId,
}: MediaListProps) {
  return (
    <Table>
      <TableHeader>
        <TableRow>
          <TableHead className="w-10" />
          <TableHead>Name</TableHead>
          <TableHead>Type</TableHead>
          <TableHead>Size</TableHead>
          <TableHead>Date</TableHead>
          <TableHead className="w-10" />
        </TableRow>
      </TableHeader>
      <TableBody>
        {files.map((file) => {
          const Icon = getCategoryIcon(file.mimetype);
          const isSelected = selectedId === file.id;

          return (
            <TableRow
              key={file.id}
              className={`cursor-pointer ${isSelected ? "bg-accent" : ""}`}
              onClick={() => onSelect?.(file)}
            >
              <TableCell>
                <div className="size-10 rounded bg-muted flex items-center justify-center overflow-hidden">
                  {isImageMime(file.mimetype) ? (
                    <img
                      src={file.url}
                      alt={file.filename}
                      width={40}
                      height={40}
                      className="object-cover size-10"
                    />
                  ) : (
                    <Icon className="size-5 text-muted-foreground" />
                  )}
                </div>
              </TableCell>
              <TableCell className="font-medium max-w-[200px] truncate">
                {file.filename}
              </TableCell>
              <TableCell className="text-muted-foreground text-xs">
                {file.mimetype}
              </TableCell>
              <TableCell className="text-muted-foreground">
                {formatFileSize(file.size)}
              </TableCell>
              <TableCell className="text-muted-foreground text-sm">
                {new Date(file.created_at).toLocaleDateString()}
              </TableCell>
              <TableCell>
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
              </TableCell>
            </TableRow>
          );
        })}
      </TableBody>
    </Table>
  );
}
