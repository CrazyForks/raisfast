"use client";

import React, { useState, useRef, useEffect, useCallback } from "react";
import { useEditor, EditorContent } from "@tiptap/react";
import StarterKit from "@tiptap/starter-kit";
import Placeholder from "@tiptap/extension-placeholder";
import Image from "@tiptap/extension-image";
import { Table, TableRow, TableCell, TableHeader } from "@tiptap/extension-table";
import {
  Bold,
  Italic,
  Strikethrough,
  Code,
  List,
  ListOrdered,
  Quote,
  CodeSquare,
  LinkIcon,
  Unlink,
  Undo,
  Redo,
  ExternalLink,
  ChevronDown,
  Heading1,
  ImageIcon,
  Code2,
} from "lucide-react";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Button } from "@/components/ui/button";
import { cn } from "@/lib/utils";
import { markdownToHtml, htmlToMarkdown } from "@/lib/markdown";

interface RichTextEditorProps {
  markdown: string;
  onChange: (markdown: string) => void;
  placeholder?: string;
  className?: string;
}

function LinkPopover({
  editor,
}: {
  editor: ReturnType<typeof useEditor> | null;
}) {
  const [open, setOpen] = useState(false);
  const [url, setUrl] = useState("");
  const [text, setText] = useState("");
  const [mode, setMode] = useState<"add" | "edit">("add");
  const ref = useRef<HTMLDivElement>(null);

  const handleOpen = useCallback(() => {
    if (!editor) return;
    const { from, to } = editor.state.selection;
    const selectedText = editor.state.doc.textBetween(from, to, "\n");
    const previousUrl = editor.getAttributes("link").href as string | undefined;
    if (previousUrl) {
      setUrl(previousUrl);
      setText(selectedText || "");
      setMode("edit");
    } else {
      setUrl("");
      setText(selectedText || "");
      setMode("add");
    }
    setOpen(true);
  }, [editor]);

  const handleSubmit = useCallback(() => {
    if (!editor || !url.trim()) return;
    const { from, to } = editor.state.selection;
    const hasSelection = from !== to;
    const chain = editor.chain().focus();

    if (mode === "add" && !hasSelection && text.trim()) {
      chain
        .insertContent({
          type: "text",
          marks: [{ type: "link", attrs: { href: url.trim() } }],
          text: text.trim(),
        })
        .run();
    } else if (hasSelection) {
      chain
        .extendMarkRange("link")
        .setLink({ href: url.trim() })
        .run();
    } else if (mode === "edit") {
      chain
        .extendMarkRange("link")
        .setLink({ href: url.trim() })
        .run();
    } else if (text.trim()) {
      chain
        .insertContent({
          type: "text",
          marks: [{ type: "link", attrs: { href: url.trim() } }],
          text: text.trim(),
        })
        .run();
    }
    setOpen(false);
  }, [editor, url, text, mode]);

  const handleRemove = useCallback(() => {
    if (!editor) return;
    editor.chain().focus().extendMarkRange("link").unsetLink().run();
    setOpen(false);
  }, [editor]);

  useEffect(() => {
    function handleClickOutside(e: MouseEvent) {
      if (ref.current && !ref.current.contains(e.target as Node)) {
        setOpen(false);
      }
    }
    if (open) document.addEventListener("mousedown", handleClickOutside);
    return () => document.removeEventListener("mousedown", handleClickOutside);
  }, [open]);

  useEffect(() => {
    function handleKeyDown(e: KeyboardEvent) {
      if (e.key === "Enter" && open) {
        e.preventDefault();
        handleSubmit();
      }
      if (e.key === "Escape" && open) {
        setOpen(false);
      }
    }
    if (open) document.addEventListener("keydown", handleKeyDown);
    return () => document.removeEventListener("keydown", handleKeyDown);
  }, [open, handleSubmit]);

  if (!editor) return null;

  const isLink = editor.isActive("link");

  return (
    <div className="relative" ref={ref}>
      <Button
        type="button"
        variant="ghost"
        size="sm"
        className={cn("h-7 w-7 p-0", isLink && "bg-accent text-accent-foreground")}
        onClick={isLink && !open ? handleOpen : isLink ? () => setOpen(!open) : handleOpen}
        title="Link"
      >
        {isLink ? <Unlink className="h-4 w-4" /> : <LinkIcon className="h-4 w-4" />}
      </Button>
      {open && (
        <div className="absolute left-0 top-full z-50 mt-1 w-80 space-y-1.5 rounded-lg border bg-popover p-2.5 shadow-lg">
          {mode === "add" && (
            <input
              type="text"
              value={text}
              onChange={(e) => setText(e.target.value)}
              placeholder="Text to display"
              className="h-8 w-full rounded-md border bg-transparent px-2.5 text-sm outline-none focus:border-primary"
            />
          )}
          <div className="flex items-center gap-1.5">
            <input
              type="url"
              value={url}
              onChange={(e) => setUrl(e.target.value)}
              placeholder="Paste or type a link..."
              className="h-8 flex-1 rounded-md border bg-transparent px-2.5 text-sm outline-none focus:border-primary"
              autoFocus={mode === "edit"}
            />
            {mode === "edit" && url && (
              <a
                href={url}
                target="_blank"
                rel="noopener noreferrer"
                className="flex h-8 w-8 shrink-0 items-center justify-center rounded-md text-muted-foreground hover:bg-accent hover:text-accent-foreground"
                title="Open link"
              >
                <ExternalLink className="h-4 w-4" />
              </a>
            )}
            <Button
              type="button"
              size="sm"
              className="h-8 shrink-0 px-3"
              disabled={!url.trim()}
              onClick={handleSubmit}
            >
              {mode === "edit" ? "Save" : "Link"}
            </Button>
            {mode === "edit" && (
              <Button
                type="button"
                variant="ghost"
                size="sm"
                className="h-8 shrink-0 px-2 text-destructive hover:text-destructive"
                onClick={handleRemove}
              >
                Remove
              </Button>
            )}
          </div>
        </div>
      )}
    </div>
  );
}

function ImagePopover({
  editor,
}: {
  editor: ReturnType<typeof useEditor> | null;
}) {
  const [open, setOpen] = useState(false);
  const [url, setUrl] = useState("");
  const [alt, setAlt] = useState("");
  const ref = useRef<HTMLDivElement>(null);

  const handleSubmit = useCallback(() => {
    if (!editor || !url.trim()) return;
    editor.chain().focus().setImage({ src: url.trim(), alt: alt.trim() }).run();
    setOpen(false);
    setUrl("");
    setAlt("");
  }, [editor, url, alt]);

  useEffect(() => {
    function handleClickOutside(e: MouseEvent) {
      if (ref.current && !ref.current.contains(e.target as Node)) {
        setOpen(false);
      }
    }
    if (open) document.addEventListener("mousedown", handleClickOutside);
    return () => document.removeEventListener("mousedown", handleClickOutside);
  }, [open]);

  useEffect(() => {
    function handleKeyDown(e: KeyboardEvent) {
      if (e.key === "Enter" && open) {
        e.preventDefault();
        handleSubmit();
      }
      if (e.key === "Escape" && open) {
        setOpen(false);
      }
    }
    if (open) document.addEventListener("keydown", handleKeyDown);
    return () => document.removeEventListener("keydown", handleKeyDown);
  }, [open, handleSubmit]);

  if (!editor) return null;

  return (
    <div className="relative" ref={ref}>
      <Button
        type="button"
        variant="ghost"
        size="sm"
        className={cn("h-7 w-7 p-0", editor.isActive("image") && "bg-accent text-accent-foreground")}
        onClick={() => setOpen(!open)}
        title="Image"
      >
        <ImageIcon className="h-4 w-4" />
      </Button>
      {open && (
        <div className="absolute left-0 top-full z-50 mt-1 w-80 space-y-1.5 rounded-lg border bg-popover p-2.5 shadow-lg">
          <input
            type="url"
            value={url}
            onChange={(e) => setUrl(e.target.value)}
            placeholder="Image URL..."
            className="h-8 w-full rounded-md border bg-transparent px-2.5 text-sm outline-none focus:border-primary"
            autoFocus
          />
          <input
            type="text"
            value={alt}
            onChange={(e) => setAlt(e.target.value)}
            placeholder="Alt text (optional)"
            className="h-8 w-full rounded-md border bg-transparent px-2.5 text-sm outline-none focus:border-primary"
          />
          <div className="flex justify-end">
            <Button
              type="button"
              size="sm"
              className="h-8 shrink-0 px-3"
              disabled={!url.trim()}
              onClick={handleSubmit}
            >
              Insert
            </Button>
          </div>
        </div>
      )}
    </div>
  );
}

function Toolbar({ editor, sourceView, onToggleSource }: { editor: ReturnType<typeof useEditor> | null; sourceView: boolean; onToggleSource: () => void }) {
  if (!editor) return null;

  const items = [
    {
      icon: Bold,
      action: () => editor.chain().focus().toggleBold().run(),
      active: editor.isActive("bold"),
      title: "Bold",
    },
    {
      icon: Italic,
      action: () => editor.chain().focus().toggleItalic().run(),
      active: editor.isActive("italic"),
      title: "Italic",
    },
    {
      icon: Strikethrough,
      action: () => editor.chain().focus().toggleStrike().run(),
      active: editor.isActive("strike"),
      title: "Strikethrough",
    },
    {
      icon: Code,
      action: () => editor.chain().focus().toggleCode().run(),
      active: editor.isActive("code"),
      title: "Inline Code",
    },
    { type: "separator" as const },
    {
      type: "heading-select" as const,
    },
    { type: "separator" as const },
    {
      icon: List,
      action: () => editor.chain().focus().toggleBulletList().run(),
      active: editor.isActive("bulletList"),
      title: "Bullet List",
    },
    {
      icon: ListOrdered,
      action: () => editor.chain().focus().toggleOrderedList().run(),
      active: editor.isActive("orderedList"),
      title: "Ordered List",
    },
    {
      icon: Quote,
      action: () => editor.chain().focus().toggleBlockquote().run(),
      active: editor.isActive("blockquote"),
      title: "Quote",
    },
    {
      icon: CodeSquare,
      action: () => editor.chain().focus().toggleCodeBlock().run(),
      active: editor.isActive("codeBlock"),
      title: "Code Block",
    },
    { type: "link-popover" as const },
    { type: "image-popover" as const },
    { type: "separator" as const },
    {
      icon: Undo,
      action: () => editor.chain().focus().undo().run(),
      active: false,
      title: "Undo",
    },
    {
      icon: Redo,
      action: () => editor.chain().focus().redo().run(),
      active: false,
      title: "Redo",
    },
  ];

  const headingLevel = editor.isActive("heading", { level: 1 })
    ? "1"
    : editor.isActive("heading", { level: 2 })
      ? "2"
      : editor.isActive("heading", { level: 3 })
        ? "3"
        : editor.isActive("heading", { level: 4 })
          ? "4"
          : editor.isActive("heading", { level: 5 })
            ? "5"
            : editor.isActive("heading", { level: 6 })
              ? "6"
              : "0";

  return (
    <div className="flex flex-wrap items-center gap-0.5 border-b px-2 py-1">
      {items.map((item, i) => {
        if ("type" in item && item.type === "separator") {
          return <div key={i} className="mx-1 h-5 w-px bg-border" />;
        }
        if ("type" in item && item.type === "heading-select") {
          if (sourceView) return null;
          return (
            <Select
              key={i}
              value={headingLevel}
              onValueChange={(v) => {
                if (v === "0") {
                  editor.chain().focus().setParagraph().run();
                } else {
                  editor.chain().focus().toggleHeading({ level: Number(v) as 1 | 2 | 3 | 4 | 5 | 6 }).run();
                }
              }}
            >
              <SelectTrigger className="h-7 w-auto border-none px-2 text-xs shadow-none">
                <SelectValue>
                  {(value: string | null) => {
                    if (value === "0") return "Paragraph";
                    if (value === "1") return "Heading 1";
                    if (value === "2") return "Heading 2";
                    if (value === "3") return "Heading 3";
                    if (value === "4") return "Heading 4";
                    if (value === "5") return "Heading 5";
                    if (value === "6") return "Heading 6";
                    return "Paragraph";
                  }}
                </SelectValue>
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="0">Paragraph</SelectItem>
                <SelectItem value="1">Heading 1</SelectItem>
                <SelectItem value="2">Heading 2</SelectItem>
                <SelectItem value="3">Heading 3</SelectItem>
                <SelectItem value="4">Heading 4</SelectItem>
                <SelectItem value="5">Heading 5</SelectItem>
                <SelectItem value="6">Heading 6</SelectItem>
              </SelectContent>
            </Select>
          );
        }
        if ("type" in item && item.type === "link-popover") {
          if (sourceView) return null;
          return <LinkPopover key={i} editor={editor} />;
        }
        if ("type" in item && item.type === "image-popover") {
          if (sourceView) return null;
          return <ImagePopover key={i} editor={editor} />;
        }
        if (sourceView) return null;
        const Icon = item.icon;
        return (
          <Button
            key={i}
            type="button"
            variant="ghost"
            size="sm"
            className={cn("h-7 w-7 p-0", item.active && "bg-accent text-accent-foreground")}
            onClick={item.action}
            title={item.title}
          >
            <Icon className="h-4 w-4" />
          </Button>
        );
      })}
      <div className="ml-auto">
        <Button
          type="button"
          variant="ghost"
          size="sm"
          className={cn("h-7 gap-1 px-2 text-xs", sourceView && "bg-accent text-accent-foreground")}
          onClick={onToggleSource}
          title={sourceView ? "Rich text" : "Markdown source"}
        >
          <Code2 className="h-3.5 w-3.5" />
          {sourceView ? "Editor" : "Source"}
        </Button>
      </div>
    </div>
  );
}

export function RichTextEditor({
  markdown,
  onChange,
  placeholder = "Write something...",
  className,
}: RichTextEditorProps) {
  const [sourceView, setSourceView] = useState(false);
  const editor = useEditor({
    extensions: [
      StarterKit.configure({
        heading: { levels: [1, 2, 3, 4, 5, 6] },
        link: {
          openOnClick: false,
          HTMLAttributes: { class: "text-primary underline" },
        },
      }),
      Placeholder.configure({
        placeholder,
      }),
      Image.configure({
        HTMLAttributes: { class: "max-w-full rounded-md" },
      }),
      Table.configure({
        resizable: true,
        HTMLAttributes: { class: "border-collapse w-full" },
      }),
      TableRow,
      TableCell.configure({
        HTMLAttributes: { class: "border px-2 py-1" },
      }),
      TableHeader.configure({
        HTMLAttributes: { class: "border px-2 py-1 bg-muted font-semibold" },
      }),
    ],
    content: markdown ? markdownToHtml(markdown) : "",
    onUpdate: ({ editor }) => {
      onChange(htmlToMarkdown(editor.getHTML()));
    },
    editorProps: {
      attributes: {
        class:
          "prose prose-sm dark:prose-invert max-w-none px-3 py-2 min-h-[120px] focus:outline-none",
      },
      handlePaste: (view, event) => {
        const text = event.clipboardData?.getData("text/plain");
        if (!text || event.clipboardData?.getData("text/html")) return false;
        const hasMarkdown = /(^|\n)#{1,6}\s|(\*\*|__)[^*_]+\1|^\s*[-*+]\s|^\s*\d+\.\s|^\s*>|\[.+\]\(.+\)|^---$|^\|.+\|$/m.test(text);
        if (!hasMarkdown) return false;
        event.preventDefault();
        const html = markdownToHtml(text);
        editor?.commands.insertContent(html);
        return true;
      },
    },
    immediatelyRender: false,
  });

  function handleToggleSource() {
    if (!sourceView && editor) {
      editor.commands.setContent(markdownToHtml(markdown));
    }
    setSourceView(!sourceView);
  }

  return (
    <div
      className={cn(
        "rounded-lg border bg-background",
        className,
      )}
    >
      <Toolbar editor={editor} sourceView={sourceView} onToggleSource={handleToggleSource} />
      {sourceView ? (
        <textarea
          value={markdown}
          onChange={(e) => onChange(e.target.value)}
          placeholder={placeholder}
          className="w-full resize-y px-3 py-2 min-h-[120px] bg-transparent text-sm font-mono outline-none"
        />
      ) : (
        <EditorContent editor={editor} />
      )}
    </div>
  );
}
