"use client";

import type { MediaFile } from "@/lib/api";
import { Copy, Download, Trash2 } from "lucide-react";
import { toast } from "sonner";

import { Button } from "@/components/ui/button";
import { Separator } from "@/components/ui/separator";
import { formatFileSize } from "./media-utils";
import { MediaPreview } from "./media-preview";
import { useT } from "@/lib/i18n";

interface MediaDetailPanelProps {
  file: MediaFile;
  onDelete: (id: string) => void;
  onClose: () => void;
}

export function MediaDetailPanel({
  file,
  onDelete,
  onClose,
}: MediaDetailPanelProps) {
  const { t } = useT();
  return (
    <div className="w-72 shrink-0 border-l bg-card overflow-y-auto space-y-4 p-4">
      <div className="flex items-center justify-between">
        <h3 className="font-semibold text-sm truncate" title={file.filename}>
          {file.filename}
        </h3>
        <button
          onClick={onClose}
          className="text-muted-foreground hover:text-foreground text-lg leading-none"
        >
          &times;
        </button>
      </div>

      <MediaPreview file={file} />

      <Separator />

      <dl className="space-y-2 text-sm">
        <div className="flex justify-between">
          <dt className="text-muted-foreground">{t("media.type")}</dt>
          <dd className="font-mono text-xs">{file.mimetype}</dd>
        </div>
        <div className="flex justify-between">
          <dt className="text-muted-foreground">{t("media.size")}</dt>
          <dd>{formatFileSize(file.size)}</dd>
        </div>
        {file.width != null && file.height != null && (
          <div className="flex justify-between">
            <dt className="text-muted-foreground">{t("media.dimensions")}</dt>
            <dd>{file.width} &times; {file.height}</dd>
          </div>
        )}
        <div className="flex justify-between">
          <dt className="text-muted-foreground">{t("media.uploaded")}</dt>
          <dd>{new Date(file.created_at).toLocaleString()}</dd>
        </div>
      </dl>

      <Separator />

      <div>
        <dt className="text-xs text-muted-foreground mb-1">{t("media.urlLabel")}</dt>
        <div className="flex gap-1">
          <code className="flex-1 text-xs bg-muted px-2 py-1.5 rounded truncate">
            {file.url}
          </code>
          <Button
            variant="outline"
            size="icon-xs"
            onClick={() => {
              navigator.clipboard.writeText(file.url);
              toast.success(t("media.urlCopied"));
            }}
          >
            <Copy className="size-3" />
          </Button>
        </div>
      </div>

      <Separator />

      <div className="flex gap-2">
        <Button
          variant="outline"
          size="sm"
          className="flex-1"
          onClick={() => window.open(file.url, "_blank")}
        >
          <Download className="size-4" />
          {t("media.download")}
        </Button>
        <Button
          variant="destructive"
          size="sm"
          className="flex-1"
          onClick={() => onDelete(file.id)}
        >
          <Trash2 className="size-4" />
          {t("common.delete")}
        </Button>
      </div>
    </div>
  );
}
