"use client";

import { useEffect, useRef, useState, useCallback } from "react";
import { createPortal } from "react-dom";
import dynamic from "next/dynamic";
import {
  Bold,
  Italic,
  Strikethrough,
  Minus,
  List,
  ListOrdered,
  ListChecks,
  Link,
  ImageIcon,
  Code,
  FileCode,
  Quote,
  Table,
  Heading,
  Video,
  FileSpreadsheet,
  FileText,
  FileType2,
} from "lucide-react";

const Icon = ({ children }: { children: React.ReactNode }) => (
  <span style={{ display: "inline-flex", alignItems: "center", width: 14, height: 14 }}>
    {children}
  </span>
);

interface MdEditorModule {
  default: any;
  bold: any;
  italic: any;
  strikethrough: any;
  hr: any;
  divider: any;
  unorderedListCommand: any;
  orderedListCommand: any;
  checkedListCommand: any;
  codeBlock: any;
  code: any;
  quote: any;
  codeEdit: any;
  codeLive: any;
  codePreview: any;
  fullscreen: any;
}

const MdEditorPromise = import(
  "@uiw/react-md-editor"
) as Promise<MdEditorModule>;

const MDEditor = dynamic(
  () =>
    MdEditorPromise.then(
      (m) => m.default
    ) as Promise<React.ComponentType<any>>,
  { ssr: false }
) as any;

import "@uiw/react-md-editor/markdown-editor.css";
import "@uiw/react-markdown-preview/markdown.css";

import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogFooter,
} from "@/components/ui/dialog";
import { MediaSelector } from "@/components/common/media-selector";
import type { MediaFile } from "@/lib/api";
import type { FileCategory } from "@/components/admin/media/media-utils";

const MAX_TABLE = 10;
const HEADING_LEVELS = [1, 2, 3, 4, 5, 6] as const;

const lucideIcons: Record<string, React.ReactNode> = {
  bold: <Icon><Bold size={14} /></Icon>,
  italic: <Icon><Italic size={14} /></Icon>,
  strikethrough: <Icon><Strikethrough size={14} /></Icon>,
  hr: <Icon><Minus size={14} /></Icon>,
  "unordered-list": <Icon><List size={14} /></Icon>,
  "ordered-list": <Icon><ListOrdered size={14} /></Icon>,
  "checked-list": <Icon><ListChecks size={14} /></Icon>,
  code: <Icon><Code size={14} /></Icon>,
  codeBlock: <Icon><FileCode size={14} /></Icon>,
  quote: <Icon><Quote size={14} /></Icon>,
};

interface MarkdownEditorProps {
  value: string;
  onChange: (value: string) => void;
  placeholder?: string;
}

type PendingAction = {
  type: "link" | "image" | "video" | "pdf" | "excel" | "word";
  textApi: any;
};

function generateTable(rows: number, cols: number): string {
  const header = Array.from({ length: cols }, (_, i) => `Header ${i + 1}`);
  const separator = Array.from({ length: cols }, () => "---");
  const body = Array.from({ length: rows - 1 }, (_, r) =>
    Array.from({ length: cols }, (_, c) => `Cell ${r + 1}-${c + 1}`)
  );
  return [header, separator, ...body]
    .map((row) => `| ${row.join(" | ")} |`)
    .join("\n");
}

function DropdownMenu({
  trigger,
  disabled,
  onOpen,
  children,
}: {
  trigger: React.ReactNode;
  disabled: boolean;
  onOpen?: () => void;
  children: (close: () => void) => React.ReactNode;
}) {
  const [open, setOpen] = useState(false);
  const triggerRef = useRef<HTMLButtonElement>(null);
  const menuRef = useRef<HTMLDivElement>(null);
  const [pos, setPos] = useState({ top: 0, left: 0 });

  useEffect(() => {
    if (!open) return;
    const onClick = (e: MouseEvent) => {
      if (menuRef.current?.contains(e.target as Node)) return;
      if (triggerRef.current?.contains(e.target as Node)) return;
      setOpen(false);
    };
    document.addEventListener("mousedown", onClick);
    return () => document.removeEventListener("mousedown", onClick);
  }, [open]);

  const handleToggle = (e: React.MouseEvent) => {
    e.preventDefault();
    e.stopPropagation();
    if (disabled) return;
    if (!open && triggerRef.current) {
      const rect = triggerRef.current.getBoundingClientRect();
      setPos({ top: rect.bottom + 4, left: rect.left });
      onOpen?.();
    }
    setOpen((v) => !v);
  };

  const close = useCallback(() => setOpen(false), []);

  return (
    <>
      <button
        ref={triggerRef}
        type="button"
        onMouseDown={handleToggle}
        disabled={disabled}
        style={{
          background: "none",
          border: "none",
          padding: "4px",
          margin: "0 1px",
          borderRadius: 2,
          cursor: disabled ? "not-allowed" : "pointer",
          color: "var(--color-fg-default)",
          display: "inline-flex",
          alignItems: "center",
          lineHeight: "14px",
          height: 20,
          outline: "none",
          transition: "all 0.3s",
        }}
      >
        {trigger}
      </button>
      {open &&
        createPortal(
          <div
            ref={menuRef}
            style={{
              position: "fixed",
              top: pos.top,
              left: pos.left,
              zIndex: 99999,
            }}
            className="rounded-lg border bg-popover shadow-lg"
          >
            {children(close)}
          </div>,
          document.body
        )}
    </>
  );
}

function TablePicker({
  onSelect,
}: {
  onSelect: (rows: number, cols: number) => void;
}) {
  const [hover, setHover] = useState<[number, number] | null>(null);
  const rows = hover ? hover[0] : -1;
  const cols = hover ? hover[1] : -1;

  return (
    <div className="p-2" onMouseLeave={() => setHover(null)}>
      <div
        className="inline-grid gap-px"
        style={{ gridTemplateColumns: `repeat(${MAX_TABLE}, 1fr)` }}
      >
        {Array.from({ length: MAX_TABLE }, (_, r) =>
          Array.from({ length: MAX_TABLE }, (_, c) => {
            const active = hover !== null && r <= rows && c <= cols;
            return (
              <div
                key={`${r}-${c}`}
                className={`size-4 border ${
                  active
                    ? "border-primary bg-primary/30"
                    : "border-border bg-muted/50"
                } cursor-pointer transition-colors`}
                onMouseEnter={() => setHover([r, c])}
                onClick={() => onSelect(r + 1, c + 1)}
              />
            );
          })
        )}
      </div>
      <div className="mt-1 text-center text-xs text-muted-foreground">
        {hover ? `${rows + 1} × ${cols + 1}` : "Select table size"}
      </div>
    </div>
  );
}

function HeadingPicker({
  onSelect,
}: {
  onSelect: (level: number) => void;
}) {
  const [hovered, setHovered] = useState<number | null>(null);
  const sizes: Record<number, string> = {
    1: "text-lg font-bold",
    2: "text-base font-bold",
    3: "text-sm font-bold",
    4: "text-sm font-semibold",
    5: "text-xs font-semibold",
    6: "text-xs font-medium",
  };

  return (
    <div className="p-1" style={{ minWidth: 120 }}>
      {HEADING_LEVELS.map((level) => (
        <button
          key={level}
          type="button"
          className={`w-full rounded-sm px-3 py-1.5 text-left transition-colors ${
            hovered === level
              ? "bg-accent text-accent-foreground"
              : "text-foreground"
          }`}
          onMouseEnter={() => setHovered(level)}
          onMouseLeave={() => setHovered(null)}
          onClick={() => onSelect(level)}
        >
          <span className={sizes[level]}>H{level}</span>
          <span className="ml-2 text-xs text-muted-foreground">
            {"#".repeat(level)}
          </span>
        </button>
      ))}
    </div>
  );
}

function buildHeadingCommand(
  apiRef: React.MutableRefObject<any>,
  stateRef: React.MutableRefObject<any>
): any {
  return {
    name: "heading",
    keyCommand: "heading",
    execute: (state: any, api: any) => {
      apiRef.current = api;
      stateRef.current = state;
    },
    render: (command: any, disabled: boolean, executeCommand: any) => (
      <DropdownMenu
        key="heading"
        trigger={<Icon><Heading size={14} /></Icon>}
        disabled={disabled}
        onOpen={() => executeCommand(command)}
      >
        {(close) => (
          <HeadingPicker
            onSelect={(level) => {
              const api = apiRef.current;
              const state = stateRef.current;
              if (!api || !state) return;
              const text = state.text;
              const sel = state.selection;
              const lineStart =
                text.lastIndexOf("\n", sel.start - 1) + 1;
              let lineEnd = text.indexOf("\n", sel.start);
              if (lineEnd === -1) lineEnd = text.length;
              const line = text.slice(lineStart, lineEnd);
              const stripped = line.replace(/^#{1,6}\s*/, "");
              const prefix = "#".repeat(level) + " ";
              api.setSelectionRange({
                start: lineStart,
                end: lineEnd,
              });
              api.replaceSelection(prefix + stripped);
              close();
            }}
          />
        )}
      </DropdownMenu>
    ),
  };
}

function buildTableCommand(
  apiRef: React.MutableRefObject<any>
): any {
  return {
    name: "table",
    keyCommand: "table",
    execute: (_state: any, api: any) => {
      apiRef.current = api;
    },
    render: (command: any, disabled: boolean, executeCommand: any) => (
      <DropdownMenu
        key="table"
        trigger={<Icon><Table size={14} /></Icon>}
        disabled={disabled}
        onOpen={() => executeCommand(command)}
      >
        {(close) => (
          <TablePicker
            onSelect={(rows, cols) => {
              apiRef.current?.replaceSelection(
                generateTable(rows, cols)
              );
              close();
            }}
          />
        )}
      </DropdownMenu>
    ),
  };
}

export function MarkdownEditor({
  value,
  onChange,
  placeholder,
}: MarkdownEditorProps) {
  const [colorMode, setColorMode] = useState<"light" | "dark">("light");
  const [dialogOpen, setDialogOpen] = useState(false);
  const [dialogType, setDialogType] = useState<PendingAction["type"]>("link");
  const [inputText, setInputText] = useState("");
  const [inputUrl, setInputUrl] = useState("");
  const [showMediaPicker, setShowMediaPicker] = useState(false);
  const pendingRef = useRef<PendingAction | null>(null);
  const apiRef = useRef<any>(null);
  const stateRef = useRef<any>(null);
  const [commands, setCommands] = useState<any[]>([]);
  const [extraCommands, setExtraCommands] = useState<any[]>([]);

  useEffect(() => {
    const sync = () => {
      setColorMode(
        document.documentElement.classList.contains("dark")
          ? "dark"
          : "light"
      );
    };
    sync();
    const obs = new MutationObserver(sync);
    obs.observe(document.documentElement, {
      attributes: true,
      attributeFilter: ["class"],
    });
    return () => obs.disconnect();
  }, []);

  useEffect(() => {
    MdEditorPromise.then((m) => {
      const linkCmd = makeDialogCommand("link");
      const imageCmd = makeDialogCommand("image");
      const videoCmd = makeDialogCommand("video");
      const pdfCmd = makeDialogCommand("pdf");
      const excelCmd = makeDialogCommand("excel");
      const wordCmd = makeDialogCommand("word");
      const headingCmd = buildHeadingCommand(apiRef, stateRef);
      const tableCmd = buildTableCommand(apiRef);
      setCommands([
        headingCmd,
        m.divider,
        m.bold,
        m.italic,
        m.strikethrough,
        m.hr,
        m.divider,
        m.unorderedListCommand,
        m.orderedListCommand,
        m.checkedListCommand,
        m.divider,
        linkCmd,
        imageCmd,
        videoCmd,
        m.divider,
        pdfCmd,
        excelCmd,
        wordCmd,
        m.divider,
        m.code,
        m.codeBlock,
        m.quote,
        tableCmd,
      ]);
      setExtraCommands([
        m.codeEdit,
        m.codeLive,
        m.codePreview,
        m.fullscreen,
      ]);
    });
  }, []);

  const applyInsert = useCallback(() => {
    const pending = pendingRef.current;
    if (!pending) return;
    if (pending.type === "video") {
      const url = inputUrl;
      let md: string;
      if (/youtu\.?be/.test(url)) {
        const vid =
          url.match(/(?:v=|youtu\.be\/|embed\/)([\w-]+)/)?.[1] || "";
        md = `<iframe width="560" height="315" src="https://www.youtube.com/embed/${vid}" frameborder="0" allowfullscreen></iframe>`;
      } else if (/bilibili\.com/.test(url)) {
        const bvid =
          url.match(/\/(BV[\w]+)/)?.[1] || "";
        md = `<iframe width="560" height="315" src="https://player.bilibili.com/player.html?bvid=${bvid}" frameborder="0" allowfullscreen></iframe>`;
      } else {
        md = `<video width="560" height="315" controls><source src="${url}" type="video/mp4"></video>`;
      }
      pending.textApi.replaceSelection(md);
      pendingRef.current = null;
      setDialogOpen(false);
      return;
    }
    if (pending.type === "pdf") {
      const md = `<iframe width="100%" height="600" src="${inputUrl}" frameborder="0"></iframe>`;
      pending.textApi.replaceSelection(md);
      pendingRef.current = null;
      setDialogOpen(false);
      return;
    }
    if (pending.type === "excel" || pending.type === "word") {
      const label = inputText || pending.type.toUpperCase();
      const md = `[${label}](${inputUrl})`;
      pending.textApi.replaceSelection(md);
      pendingRef.current = null;
      setDialogOpen(false);
      return;
    }
    const text =
      inputText || (pending.type === "link" ? "link" : "image");
    const url = inputUrl;
    const wrap = pending.type === "link" ? "" : "!";
    const md = `${wrap}[${text}](${url})`;
    pending.textApi.replaceSelection(md);
    pendingRef.current = null;
    setDialogOpen(false);
  }, [inputText, inputUrl]);

  const handleMediaSelect = useCallback((file: MediaFile) => {
    setInputUrl(file.url);
    if (!inputText) {
      setInputText(file.filename.replace(/\.[^.]+$/, ""));
    }
    setShowMediaPicker(false);
  }, [inputText]);

  const handleCancel = useCallback(() => {
    pendingRef.current = null;
    setDialogOpen(false);
  }, []);

  function makeDialogCommand(type: PendingAction["type"]) {
    const labels: Record<string, { aria: string; title: string; shortcut?: string }> = {
      link: { aria: "Insert Link", title: "Insert Link (Ctrl+L)", shortcut: "ctrlcmd+l" },
      image: { aria: "Insert Image", title: "Insert Image (Ctrl+K)", shortcut: "ctrlcmd+k" },
      video: { aria: "Insert Video", title: "Insert Video" },
      pdf: { aria: "Insert PDF", title: "Insert PDF" },
      excel: { aria: "Insert Excel", title: "Insert Excel" },
      word: { aria: "Insert Word", title: "Insert Word" },
    };
    const icons: Record<string, React.ReactNode> = {
      link: <Icon><Link size={14} /></Icon>,
      image: <Icon><ImageIcon size={14} /></Icon>,
      video: <Icon><Video size={14} /></Icon>,
      pdf: <Icon><FileText size={14} /></Icon>,
      excel: <Icon><FileSpreadsheet size={14} /></Icon>,
      word: <Icon><FileType2 size={14} /></Icon>,
    };
    const cmd: any = {
      name: type,
      keyCommand: type,
      buttonProps: {
        "aria-label": labels[type].aria,
        title: labels[type].title,
      },
      icon: icons[type],
      execute: (state: any, api: any) => {
        pendingRef.current = { type, textApi: api };
        setInputText(state.selectedText || "");
        setInputUrl("");
        setDialogType(type);
        setShowMediaPicker(false);
        setDialogOpen(true);
      },
    };
    if (labels[type].shortcut) cmd.shortcuts = labels[type].shortcut;
    return cmd;
  }

  return (
    <div data-color-mode={colorMode}>
      <MDEditor
        value={value}
        onChange={(v: any) => onChange(v ?? "")}
        textareaProps={{ placeholder }}
        height={400}
        preview="live"
        visibleDragbar={false}
        commands={commands}
        extraCommands={extraCommands}
        commandsFilter={(command: any) => {
          const icon = lucideIcons[command.name];
          if (icon) return { ...command, icon };
          return command;
        }}
      />
      <Dialog
        open={dialogOpen}
        onOpenChange={(o: boolean) => !o && handleCancel()}
      >
        <DialogContent>
          <DialogHeader>
            <DialogTitle>
              {dialogType === "link"
                ? "Insert Link"
                : dialogType === "image"
                  ? "Insert Image"
                  : dialogType === "video"
                    ? "Insert Video"
                    : dialogType === "pdf"
                      ? "Insert PDF"
                      : dialogType === "excel"
                        ? "Insert Excel"
                        : "Insert Word"}
            </DialogTitle>
          </DialogHeader>
          <div className="space-y-3">
            {!["video", "pdf"].includes(dialogType) && (
              <div className="space-y-1.5">
                <Label htmlFor="md-text">
                  {dialogType === "link" ? "Link Text" : "Alt Text"}
                </Label>
                <Input
                  id="md-text"
                  value={inputText}
                  onChange={(e) => setInputText(e.target.value)}
                  placeholder={
                    dialogType === "link"
                      ? "Display text"
                      : "Description"
                  }
                  autoFocus
                  onKeyDown={(e) =>
                    e.key === "Enter" && applyInsert()
                  }
                />
              </div>
            )}
            <div className="space-y-1.5">
              <Label htmlFor="md-url">URL</Label>
              <Input
                id="md-url"
                value={inputUrl}
                onChange={(e) => setInputUrl(e.target.value)}
                placeholder={
                  dialogType === "video"
                    ? "Video URL or YouTube / Bilibili link"
                    : dialogType === "pdf"
                      ? "PDF file URL"
                      : dialogType === "excel"
                        ? "Excel file URL"
                        : dialogType === "word"
                          ? "Word file URL"
                          : dialogType === "link"
                            ? "https://example.com"
                            : "https://example.com/image.png"
                }
                autoFocus={["video", "pdf", "excel", "word"].includes(dialogType)}
                onKeyDown={(e) =>
                  e.key === "Enter" && applyInsert()
                }
              />
              {dialogType === "video" && (
                <p className="text-xs text-muted-foreground">
                  Supports MP4 links, YouTube, and Bilibili
                </p>
              )}
            </div>
            {["image", "pdf", "excel", "word"].includes(dialogType) && (
              <div>
                {showMediaPicker ? (
                  <MediaSelector
                    onSelect={handleMediaSelect}
                    onClose={() => setShowMediaPicker(false)}
                    category={
                      dialogType === "image" ? "image"
                        : dialogType === "pdf" ? "document"
                          : dialogType === "excel" ? "spreadsheet"
                            : "document"
                    }
                  />
                ) : (
                  <Button
                    variant="outline"
                    size="sm"
                    className="w-full"
                    onClick={() => setShowMediaPicker(true)}
                  >
                    <ImageIcon className="size-4" />
                    Select from Media
                  </Button>
                )}
              </div>
            )}
          </div>
          <DialogFooter>
            <Button variant="outline" onClick={handleCancel}>
              Cancel
            </Button>
            <Button onClick={applyInsert}>Insert</Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </div>
  );
}
