"use client";

import type { MediaFile } from "@/lib/api";
import { FILE_CATEGORIES, matchesCategory, type FileCategory } from "./media-utils";
import { cn } from "@/lib/utils";

interface MediaSidebarProps {
  files: MediaFile[];
  selected: FileCategory;
  onSelect: (cat: FileCategory) => void;
}

export function MediaSidebar({ files, selected, onSelect }: MediaSidebarProps) {
  const counts = files.reduce(
    (acc, f) => {
      const cat = matchesCategory(f, "all") ? "all" : "all";
      acc["all"] = (acc["all"] ?? 0) + 1;
      for (const c of FILE_CATEGORIES) {
        if (matchesCategory(f, c.key)) {
          acc[c.key] = (acc[c.key] ?? 0) + 1;
        }
      }
      return acc;
    },
    {} as Record<string, number>,
  );

  return (
    <div className="space-y-1">
      <SidebarItem
        label="All"
        count={counts["all"] ?? 0}
        active={selected === "all"}
        onClick={() => onSelect("all")}
      />
      {FILE_CATEGORIES.map((cat) => {
        const Icon = cat.icon;
        return (
          <SidebarItem
            key={cat.key}
            icon={<Icon className="size-4" />}
            label={cat.label}
            count={counts[cat.key] ?? 0}
            active={selected === cat.key}
            onClick={() => onSelect(cat.key)}
          />
        );
      })}
      <SidebarItem
        label="Other"
        count={
          files.length -
          FILE_CATEGORIES.reduce(
            (n, c) => n + (counts[c.key] ?? 0),
            0,
          )
        }
        active={selected === "other"}
        onClick={() => onSelect("other")}
      />
    </div>
  );
}

function SidebarItem({
  icon,
  label,
  count,
  active,
  onClick,
}: {
  icon?: React.ReactNode;
  label: string;
  count: number;
  active: boolean;
  onClick: () => void;
}) {
  return (
    <button
      onClick={onClick}
      className={cn(
        "flex w-full items-center gap-2 rounded-md px-2 py-1.5 text-sm transition-colors hover:bg-accent",
        active && "bg-accent font-medium",
      )}
    >
      {icon}
      <span className="flex-1 text-left">{label}</span>
      <span className="text-xs text-muted-foreground">{count}</span>
    </button>
  );
}
