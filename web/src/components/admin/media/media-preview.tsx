"use client";

import type { MediaFile } from "@/lib/api";
import {
  isImageMime,
  isVideoMime,
  isAudioMime,
  isPdfMime,
  getCategoryIcon,
  formatFileSize,
} from "./media-utils";
import { useT } from "@/lib/i18n";

interface MediaPreviewProps {
  file: MediaFile;
}

export function MediaPreview({ file }: MediaPreviewProps) {
  const { t } = useT();
  const Icon = getCategoryIcon(file.mimetype);

  if (isImageMime(file.mimetype)) {
    return (
      <div className="flex items-center justify-center bg-muted rounded-lg p-4 max-h-[400px] overflow-hidden">
        {/* eslint-disable-next-line @next/next/no-img-element */}
        <img
          src={file.url}
          alt={file.filename}
          className="max-w-full max-h-[380px] object-contain rounded"
        />
      </div>
    );
  }

  if (isVideoMime(file.mimetype)) {
    return (
      <div className="rounded-lg overflow-hidden bg-black">
        <video
          src={file.url}
          controls
          className="w-full max-h-[400px]"
        >
          {t("media.noVideoSupport")}
        </video>
      </div>
    );
  }

  if (isAudioMime(file.mimetype)) {
    return (
      <div className="flex items-center justify-center bg-muted rounded-lg p-8">
        <div className="text-center space-y-4">
          <Icon className="size-16 text-muted-foreground mx-auto" />
          <audio src={file.url} controls className="w-full max-w-sm" />
        </div>
      </div>
    );
  }

  if (isPdfMime(file.mimetype)) {
    return (
      <iframe
        src={file.url}
        className="w-full h-[400px] rounded-lg border"
        title={file.filename}
      />
    );
  }

  return (
    <div className="flex items-center justify-center bg-muted rounded-lg p-8">
      <div className="text-center space-y-2">
        <Icon className="size-16 text-muted-foreground mx-auto" />
        <p className="text-sm text-muted-foreground">
          {file.mimetype} &middot; {formatFileSize(file.size)}
        </p>
        <a
          href={file.url}
          target="_blank"
          rel="noopener noreferrer"
          className="text-sm text-primary underline"
        >
          {t("media.downloadFile")}
        </a>
      </div>
    </div>
  );
}
