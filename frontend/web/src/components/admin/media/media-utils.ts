import type { MediaFile } from "@raisfast/sdk";
import {
  Image,
  Video,
  Music,
  FileText,
  Table2,
  Archive,
  File,
  type LucideIcon,
} from "lucide-react";

export type FileCategory =
  | "all"
  | "image"
  | "video"
  | "audio"
  | "document"
  | "spreadsheet"
  | "archive"
  | "other";

export interface CategoryInfo {
  key: FileCategory;
  label: string;
  icon: LucideIcon;
  mimes: string[];
}

export const FILE_CATEGORIES: CategoryInfo[] = [
  {
    key: "image",
    label: "Images",
    icon: Image,
    mimes: [
      "image/jpeg",
      "image/png",
      "image/gif",
      "image/webp",
      "image/svg+xml",
    ],
  },
  {
    key: "video",
    label: "Video",
    icon: Video,
    mimes: ["video/mp4", "video/webm", "video/quicktime"],
  },
  {
    key: "audio",
    label: "Audio",
    icon: Music,
    mimes: ["audio/mpeg", "audio/ogg", "audio/wav", "audio/aac"],
  },
  {
    key: "document",
    label: "Docs",
    icon: FileText,
    mimes: [
      "application/pdf",
      "application/msword",
      "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
      "application/vnd.ms-powerpoint",
      "application/vnd.openxmlformats-officedocument.presentationml.presentation",
    ],
  },
  {
    key: "spreadsheet",
    label: "Sheets",
    icon: Table2,
    mimes: [
      "application/vnd.ms-excel",
      "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
    ],
  },
  {
    key: "archive",
    label: "Archives",
    icon: Archive,
    mimes: [
      "application/zip",
      "application/x-tar",
      "application/gzip",
      "application/x-rar-compressed",
    ],
  },
];

export function getCategoryForMime(
  mime: string,
): Exclude<FileCategory, "all"> {
  for (const cat of FILE_CATEGORIES) {
    if (cat.mimes.includes(mime))
      return cat.key as Exclude<FileCategory, "all">;
  }
  return "other";
}

export function getCategoryIcon(mime: string): LucideIcon {
  const cat = getCategoryForMime(mime);
  const info = FILE_CATEGORIES.find((c) => c.key === cat);
  return info?.icon ?? File;
}

export function matchesCategory(file: MediaFile, category: FileCategory): boolean {
  if (category === "all") return true;
  return getCategoryForMime(file.mimetype) === category;
}

export function getAcceptForCategory(category: FileCategory): string {
  if (category === "all" || category === "other") return "";
  const info = FILE_CATEGORIES.find((c) => c.key === category);
  return info?.mimes.join(",") ?? "";
}

export function formatFileSize(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  if (bytes < 1024 * 1024 * 1024)
    return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
  return `${(bytes / (1024 * 1024 * 1024)).toFixed(1)} GB`;
}

export function isImageMime(mime: string): boolean {
  return mime.startsWith("image/");
}

export function isVideoMime(mime: string): boolean {
  return mime.startsWith("video/");
}

export function isAudioMime(mime: string): boolean {
  return mime.startsWith("audio/");
}

export function isPdfMime(mime: string): boolean {
  return mime === "application/pdf";
}

type SortField = "created_at" | "filename" | "size";
type SortOrder = "asc" | "desc";

export function sortFiles(
  files: MediaFile[],
  field: SortField,
  order: SortOrder,
): MediaFile[] {
  const sorted = [...files].sort((a, b) => {
    let cmp = 0;
    switch (field) {
      case "filename":
        cmp = a.filename.localeCompare(b.filename);
        break;
      case "size":
        cmp = a.size - b.size;
        break;
      case "created_at":
      default:
        cmp =
          new Date(a.created_at).getTime() - new Date(b.created_at).getTime();
        break;
    }
    return order === "desc" ? -cmp : cmp;
  });
  return sorted;
}
