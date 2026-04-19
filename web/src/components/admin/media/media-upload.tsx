"use client";

import { useCallback, useEffect, useRef, useState } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { Upload, X } from "lucide-react";
import { toast } from "sonner";

import { Button } from "@/components/ui/button";
import { api, ApiError } from "@/lib/api";

interface UploadItem {
  id: string;
  file: File;
  progress: number;
  status: "uploading" | "done" | "error";
  error?: string;
}

export function MediaUpload({ accept = "" }: { accept?: string }) {
  const queryClient = useQueryClient();
  const fileInputRef = useRef<HTMLInputElement>(null);
  const dropRef = useRef<HTMLDivElement>(null);
  const [uploads, setUploads] = useState<UploadItem[]>([]);
  const [dragOver, setDragOver] = useState(false);

  const addFiles = useCallback(
    (files: FileList | File[]) => {
      const newItems: UploadItem[] = Array.from(files).map((file) => ({
        id: `${Date.now()}-${Math.random().toString(36).slice(2, 8)}`,
        file,
        progress: 0,
        status: "uploading" as const,
      }));
      setUploads((prev) => [...prev, ...newItems]);
      newItems.forEach((item) => {
        uploadFile(item.id, item.file);
      });
    },
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [],
  );

  async function uploadFile(itemId: string, file: File) {
    setUploads((prev) =>
      prev.map((u) =>
        u.id === itemId ? { ...u, progress: 30, status: "uploading" } : u,
      ),
    );
    try {
      await api.upload("/media/upload", file);
      setUploads((prev) =>
        prev.map((u) =>
          u.id === itemId ? { ...u, progress: 100, status: "done" } : u,
        ),
      );
      queryClient.invalidateQueries({ queryKey: ["media"] });
    } catch (err) {
      const msg = err instanceof ApiError ? err.message : "Upload failed";
      setUploads((prev) =>
        prev.map((u) =>
          u.id === itemId
            ? { ...u, progress: 0, status: "error", error: msg }
            : u,
        ),
      );
      toast.error(`${file.name}: ${msg}`);
    }
  }

  function removeUpload(id: string) {
    setUploads((prev) => prev.filter((u) => u.id !== id));
  }

  useEffect(() => {
    const el = dropRef.current;
    if (!el) return;

    function onDragOver(e: DragEvent) {
      e.preventDefault();
      setDragOver(true);
    }
    function onDragLeave() {
      setDragOver(false);
    }
    function onDrop(e: DragEvent) {
      e.preventDefault();
      setDragOver(false);
      if (e.dataTransfer?.files.length) addFiles(e.dataTransfer.files);
    }
    function onPaste(e: ClipboardEvent) {
      const items = e.clipboardData?.items;
      if (!items) return;
      const files: File[] = [];
      for (const item of items) {
        if (item.kind === "file") {
          const f = item.getAsFile();
          if (f) files.push(f);
        }
      }
      if (files.length) addFiles(files);
    }

    el.addEventListener("dragover", onDragOver);
    el.addEventListener("dragleave", onDragLeave);
    el.addEventListener("drop", onDrop);
    document.addEventListener("paste", onPaste);
    return () => {
      el.removeEventListener("dragover", onDragOver);
      el.removeEventListener("dragleave", onDragLeave);
      el.removeEventListener("drop", onDrop);
      document.removeEventListener("paste", onPaste);
    };
  }, [addFiles]);

  const activeCount = uploads.filter((u) => u.status === "uploading").length;

  return (
    <div ref={dropRef} className="space-y-2">
      <div
        onClick={() => fileInputRef.current?.click()}
        className={`flex cursor-pointer items-center justify-center gap-2 rounded-lg border-2 border-dashed p-4 transition-colors ${
          dragOver
            ? "border-primary bg-primary/5"
            : "border-muted-foreground/25 hover:border-muted-foreground/50"
        }`}
      >
        <Upload className="size-5 text-muted-foreground" />
        <span className="text-sm text-muted-foreground">
          Drop files here, paste, or <span className="text-primary underline">browse</span>
        </span>
        <input
          ref={fileInputRef}
          type="file"
          className="hidden"
          multiple
          accept={accept}
          onChange={(e) => {
            e.stopPropagation();
            if (e.target.files) addFiles(e.target.files);
            if (fileInputRef.current) fileInputRef.current.value = "";
          }}
        />
      </div>

      {activeCount > 0 && (
        <p className="text-xs text-muted-foreground text-center">
          Uploading {activeCount} file{activeCount > 1 ? "s" : ""}...
        </p>
      )}

      {uploads.length > 0 && (
        <div className="space-y-1">
          {uploads.map((item) => (
            <div
              key={item.id}
              className="flex items-center gap-2 rounded-md border bg-card px-3 py-1.5 text-sm"
            >
              <span className="flex-1 truncate">{item.file.name}</span>
              {item.status === "uploading" && (
                <div className="h-1.5 w-20 overflow-hidden rounded-full bg-muted">
                  <div
                    className="h-full bg-primary transition-all"
                    style={{ width: `${item.progress}%` }}
                  />
                </div>
              )}
              {item.status === "done" && (
                <span className="text-xs text-green-600">Done</span>
              )}
              {item.status === "error" && (
                <span className="text-xs text-destructive" title={item.error}>
                  Failed
                </span>
              )}
              <button
                onClick={() => removeUpload(item.id)}
                className="text-muted-foreground hover:text-foreground"
              >
                <X className="size-3.5" />
              </button>
            </div>
          ))}
        </div>
      )}
    </div>
  );
}
