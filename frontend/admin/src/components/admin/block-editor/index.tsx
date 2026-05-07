
import { useState, useCallback } from "react";
import { Plus, ChevronUp, ChevronDown, Trash2, GripVertical } from "lucide-react";

import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Textarea } from "@/components/ui/textarea";
import { useT } from "@/lib/i18n";
const BLOCK_TYPES = [
  { type: "hero", label: "Hero", icon: "🖼" },
  { type: "richtext", label: "Text", icon: "📝" },
  { type: "image", label: "Image", icon: "📷" },
  { type: "gallery", label: "Gallery", icon: "🖼" },
  { type: "video", label: "Video", icon: "🎬" },
  { type: "cta", label: "CTA", icon: "📢" },
  { type: "testimonial", label: "Testimonial", icon: "⭐" },
  { type: "faq", label: "FAQ", icon: "❓" },
  { type: "stats", label: "Stats", icon: "📊" },
  { type: "timeline", label: "Timeline", icon: "⏱" },
  { type: "team", label: "Team", icon: "👥" },
  { type: "pricing", label: "Pricing", icon: "💰" },
  { type: "contact_form", label: "Form", icon: "📮" },
  { type: "map", label: "Map", icon: "📍" },
  { type: "code", label: "Code", icon: "📋" },
  { type: "quote", label: "Quote", icon: "💬" },
  { type: "divider", label: "Divider", icon: "➖" },
  { type: "spacer", label: "Spacer", icon: "↕️" },
  { type: "columns", label: "Columns", icon: "📦" },
  { type: "html", label: "HTML", icon: "🌐" },
  { type: "reusable", label: "Reusable", icon: "🔄" },
];

interface BlockEditorProps {
  blocks: object[];
  onChange: (blocks: object[]) => void;
}

export function BlockEditor({ blocks, onChange }: BlockEditorProps) {
  const { t } = useT();
  const [showMenu, setShowMenu] = useState(false);
  const [insertAt, setInsertAt] = useState(-1);

  const update = useCallback(
    (idx: number, block: object) => {
      const next = [...blocks];
      next[idx] = block;
      onChange(next);
    },
    [blocks, onChange],
  );

  const remove = useCallback(
    (idx: number) => {
      onChange(blocks.filter((_, i) => i !== idx));
    },
    [blocks, onChange],
  );

  const move = useCallback(
    (idx: number, dir: -1 | 1) => {
      const target = idx + dir;
      if (target < 0 || target >= blocks.length) return;
      const next = [...blocks];
      [next[idx], next[target]] = [next[target], next[idx]];
      onChange(next);
    },
    [blocks, onChange],
  );

  function addBlock(type: string) {
    const defaults: Record<string, object> = {
      hero: { type: "hero", title: "", subtitle: "", height: "md" },
      richtext: { type: "richtext", content: "" },
      image: { type: "image", url: "", alt: "" },
      gallery: { type: "gallery", images: [] },
      video: { type: "video", url: "" },
      cta: { type: "cta", title: "", button_text: "", button_url: "" },
      testimonial: { type: "testimonial", items: [] },
      faq: { type: "faq", items: [] },
      stats: { type: "stats", items: [] },
      timeline: { type: "timeline", items: [] },
      team: { type: "team", members: [] },
      pricing: { type: "pricing", plans: [] },
      contact_form: { type: "contact_form", fields: [{ name: "name", label: "Name", field_type: "text", required: true }, { name: "email", label: "Email", field_type: "email", required: true }, { name: "message", label: "Message", field_type: "textarea", required: true }], submit_text: "" },
      map: { type: "map", address: "", lat: "", lng: "", zoom: 14 },
      code: { type: "code", code: "", language: "" },
      quote: { type: "quote", content: "" },
      divider: { type: "divider", style: "solid" },
      spacer: { type: "spacer", height: "md" },
      columns: { type: "columns", columns: [{ blocks: [] }, { blocks: [] }] },
      html: { type: "html", content: "" },
      reusable: { type: "reusable", ref_id: "" },
    };
    const block = defaults[type] ?? { type };
    const pos = insertAt >= 0 ? insertAt : blocks.length;
    const next = [...blocks];
    next.splice(pos, 0, block);
    onChange(next);
    setShowMenu(false);
    setInsertAt(-1);
  }

  function getBlockLabel(block: Record<string, unknown>): string {
    const bt = block.type as string;
    const def = BLOCK_TYPES.find((b) => b.type === bt);
    const preview = (block.title as string) || (block.content as string) || "";
    return def ? `${def.icon} ${def.label}${preview ? ": " + preview.slice(0, 30) : ""}` : bt;
  }

  return (
    <div className="space-y-3">
      {blocks.map((block, idx) => (
        <Card key={idx} className="group">
          <CardContent className="p-3">
            <div className="flex items-center gap-2 mb-2">
              <GripVertical className="size-4 text-muted-foreground cursor-grab" />
              <span className="text-sm font-medium flex-1 truncate">
                {getBlockLabel(block as Record<string, unknown>)}
              </span>
              <div className="flex gap-1 opacity-0 group-hover:opacity-100 transition-opacity">
                <Button variant="ghost" size="icon-sm" onClick={() => move(idx, -1)} disabled={idx === 0}>
                  <ChevronUp className="size-3" />
                </Button>
                <Button variant="ghost" size="icon-sm" onClick={() => move(idx, 1)} disabled={idx === blocks.length - 1}>
                  <ChevronDown className="size-3" />
                </Button>
                <Button variant="ghost" size="icon-sm" onClick={() => { setShowMenu(true); setInsertAt(idx + 1); }}>
                  <Plus className="size-3" />
                </Button>
                <Button variant="ghost" size="icon-sm" onClick={() => remove(idx)}>
                  <Trash2 className="size-3" />
                </Button>
              </div>
            </div>
            <BlockForm block={block as Record<string, unknown>} onChange={(b) => update(idx, b)} />
          </CardContent>
        </Card>
      ))}

      <Button
        variant="outline"
        className="w-full border-dashed"
        onClick={() => { setShowMenu(true); setInsertAt(-1); }}
      >
        <Plus className="size-4 mr-2" />
        {t("pages.addBlock")}
      </Button>

      {showMenu && (
        <Card className="border-primary/50">
          <CardContent className="p-3">
            <div className="flex items-center justify-between mb-2">
              <span className="text-sm font-medium">{t("pages.chooseBlock")}</span>
              <Button variant="ghost" size="icon-sm" onClick={() => { setShowMenu(false); setInsertAt(-1); }}>✕</Button>
            </div>
            <div className="grid grid-cols-4 gap-2">
              {BLOCK_TYPES.map((bt) => (
                <button
                  key={bt.type}
                  type="button"
                  onClick={() => addBlock(bt.type)}
                  className="flex items-center gap-1.5 rounded-md border px-3 py-2 text-xs hover:bg-muted transition-colors text-left"
                >
                  <span>{bt.icon}</span>
                  <span>{bt.label}</span>
                </button>
              ))}
            </div>
          </CardContent>
        </Card>
      )}
    </div>
  );
}

function BlockForm({
  block,
  onChange,
}: {
  block: Record<string, unknown>;
  onChange: (block: Record<string, unknown>) => void;
}) {
  const type = block.type as string;

  function set(key: string, value: unknown) {
    onChange({ ...block, [key]: value });
  }

  switch (type) {
    case "hero":
      return (
        <div className="space-y-2">
          <Input placeholder="Title" value={(block.title as string) ?? ""} onChange={(e) => set("title", e.target.value)} />
          <Input placeholder="Subtitle" value={(block.subtitle as string) ?? ""} onChange={(e) => set("subtitle", e.target.value)} />
          <div className="grid grid-cols-2 gap-2">
            <Input placeholder="CTA Text" value={(block.cta_text as string) ?? ""} onChange={(e) => set("cta_text", e.target.value)} />
            <Input placeholder="CTA URL" value={(block.cta_url as string) ?? ""} onChange={(e) => set("cta_url", e.target.value)} />
          </div>
          <div className="grid grid-cols-2 gap-2">
            <Input placeholder="Background Image" value={(block.background_image as string) ?? ""} onChange={(e) => set("background_image", e.target.value)} />
            <select className="flex h-9 rounded-md border border-input bg-background px-3 py-1 text-sm" value={(block.height as string) ?? "md"} onChange={(e) => set("height", e.target.value)}>
              <option value="sm">Small</option><option value="md">Medium</option><option value="lg">Large</option><option value="full">Full</option>
            </select>
          </div>
        </div>
      );
    case "richtext":
      return <Textarea rows={6} placeholder="Markdown content..." value={(block.content as string) ?? ""} onChange={(e) => set("content", e.target.value)} />;
    case "image":
      return (
        <div className="space-y-2">
          <Input placeholder="Image URL" value={(block.url as string) ?? ""} onChange={(e) => set("url", e.target.value)} />
          <div className="grid grid-cols-2 gap-2">
            <Input placeholder="Alt text" value={(block.alt as string) ?? ""} onChange={(e) => set("alt", e.target.value)} />
            <Input placeholder="Caption" value={(block.caption as string) ?? ""} onChange={(e) => set("caption", e.target.value)} />
          </div>
        </div>
      );
    case "video":
      return <Input placeholder="Video URL (YouTube / Bilibili)" value={(block.url as string) ?? ""} onChange={(e) => set("url", e.target.value)} />;
    case "cta":
      return (
        <div className="space-y-2">
          <Input placeholder="Title" value={(block.title as string) ?? ""} onChange={(e) => set("title", e.target.value)} />
          <Input placeholder="Description" value={(block.description as string) ?? ""} onChange={(e) => set("description", e.target.value)} />
          <div className="grid grid-cols-2 gap-2">
            <Input placeholder="Button Text" value={(block.button_text as string) ?? ""} onChange={(e) => set("button_text", e.target.value)} />
            <Input placeholder="Button URL" value={(block.button_url as string) ?? ""} onChange={(e) => set("button_url", e.target.value)} />
          </div>
        </div>
      );
    case "code":
      return (
        <div className="space-y-2">
          <Textarea rows={6} placeholder="Code..." value={(block.code as string) ?? ""} onChange={(e) => set("code", e.target.value)} />
          <Input placeholder="Language (js, rust, ...)" value={(block.language as string) ?? ""} onChange={(e) => set("language", e.target.value)} />
        </div>
      );
    case "quote":
      return (
        <div className="space-y-2">
          <Textarea rows={3} placeholder="Quote..." value={(block.content as string) ?? ""} onChange={(e) => set("content", e.target.value)} />
          <Input placeholder="Author" value={(block.author as string) ?? ""} onChange={(e) => set("author", e.target.value)} />
        </div>
      );
    case "html":
      return <Textarea rows={6} placeholder="<div>...</div>" value={(block.content as string) ?? ""} onChange={(e) => set("content", e.target.value)} />;
    case "divider":
      return (
        <select className="flex h-9 rounded-md border border-input bg-background px-3 py-1 text-sm" value={(block.style as string) ?? "solid"} onChange={(e) => set("style", e.target.value)}>
          <option value="solid">Solid</option><option value="dashed">Dashed</option><option value="dotted">Dotted</option><option value="space">Space</option>
        </select>
      );
    case "spacer":
      return (
        <select className="flex h-9 rounded-md border border-input bg-background px-3 py-1 text-sm" value={(block.height as string) ?? "md"} onChange={(e) => set("height", e.target.value)}>
          <option value="sm">Small</option><option value="md">Medium</option><option value="lg">Large</option><option value="xl">Extra Large</option>
        </select>
      );
    case "reusable":
      return <Input placeholder="Reusable Block ID" value={(block.ref_id as string) ?? ""} onChange={(e) => set("ref_id", e.target.value)} />;
    case "map":
      return (
        <div className="space-y-2">
          <Input placeholder="Address" value={(block.address as string) ?? ""} onChange={(e) => set("address", e.target.value)} />
          <div className="grid grid-cols-2 gap-2">
            <Input placeholder="Latitude" value={(block.lat as string) ?? ""} onChange={(e) => set("lat", e.target.value)} />
            <Input placeholder="Longitude" value={(block.lng as string) ?? ""} onChange={(e) => set("lng", e.target.value)} />
          </div>
          <div className="grid grid-cols-2 gap-2">
            <Input placeholder="Zoom (1-20)" type="number" min={1} max={20} value={(block.zoom as number) ?? 14} onChange={(e) => set("zoom", parseInt(e.target.value) || 14)} />
            <Input placeholder="Title" value={(block.title as string) ?? ""} onChange={(e) => set("title", e.target.value)} />
          </div>
        </div>
      );
    case "contact_form":
      return <ContactFormEditor block={block} onChange={onChange} />;
    case "columns":
      return <ColumnsEditor block={block} onChange={onChange} />;
    case "stats":
      return <JsonItemsEditor block={block} field="items" itemLabel="Stat" fields={["label", "value", "suffix"]} onChange={onChange} />;
    case "faq":
      return <JsonItemsEditor block={block} field="items" itemLabel="FAQ" fields={["question", "answer"]} onChange={onChange} />;
    case "testimonial":
      return <JsonItemsEditor block={block} field="items" itemLabel="Testimonial" fields={["quote", "author", "company"]} onChange={onChange} />;
    case "timeline":
      return <JsonItemsEditor block={block} field="items" itemLabel="Event" fields={["date", "title", "description"]} onChange={onChange} />;
    case "team":
      return <JsonItemsEditor block={block} field="members" itemLabel="Member" fields={["name", "role", "avatar"]} onChange={onChange} />;
    case "pricing":
      return <JsonItemsEditor block={block} field="plans" itemLabel="Plan" fields={["name", "price", "period"]} onChange={onChange} />;
    case "gallery":
      return <JsonItemsEditor block={block} field="images" itemLabel="Image" fields={["url", "alt", "caption"]} onChange={onChange} />;
    default:
      return (
        <Textarea rows={4} value={JSON.stringify(block, null, 2)} onChange={(e) => { try { onChange(JSON.parse(e.target.value)); } catch { /* ignore */ } }} />
      );
  }
}

function JsonItemsEditor({
  block,
  field,
  itemLabel,
  fields,
  onChange,
}: {
  block: Record<string, unknown>;
  field: string;
  itemLabel: string;
  fields: string[];
  onChange: (block: Record<string, unknown>) => void;
}) {
  const items = (block[field] as Record<string, string>[]) ?? [];

  function updateItem(idx: number, key: string, value: string) {
    const next = [...items];
    next[idx] = { ...next[idx], [key]: value };
    onChange({ ...block, [field]: next });
  }

  function addItem() {
    const next = [...items, Object.fromEntries(fields.map((f) => [f, ""]))];
    onChange({ ...block, [field]: next });
  }

  function removeItem(idx: number) {
    onChange({ ...block, [field]: items.filter((_, i) => i !== idx) });
  }

  return (
    <div className="space-y-2">
      {items.map((item, idx) => (
        <div key={idx} className="flex items-start gap-2 p-2 rounded border">
          <div className="flex-1 grid grid-cols-2 gap-2">
            {fields.map((f) => (
              <Input key={f} placeholder={f} value={item[f] ?? ""} onChange={(e) => updateItem(idx, f, e.target.value)} />
            ))}
          </div>
          <Button variant="ghost" size="icon-sm" onClick={() => removeItem(idx)}>
            <Trash2 className="size-3" />
          </Button>
        </div>
      ))}
      <Button variant="outline" size="sm" onClick={addItem}>
        <Plus className="size-3 mr-1" />{itemLabel}
      </Button>
    </div>
  );
}

const FIELD_TYPES = ["text", "email", "textarea", "tel", "number", "url", "select"];

function ContactFormEditor({
  block,
  onChange,
}: {
  block: Record<string, unknown>;
  onChange: (block: Record<string, unknown>) => void;
}) {
  const fields = (block.fields as { name: string; label: string; field_type: string; required?: boolean; options?: string }[]) ?? [];

  function updateField(idx: number, key: string, value: string | boolean) {
    const next = [...fields];
    next[idx] = { ...next[idx], [key]: value };
    onChange({ ...block, fields: next });
  }

  function addField() {
    onChange({ ...block, fields: [...fields, { name: "", label: "", field_type: "text", required: false }] });
  }

  function removeField(idx: number) {
    onChange({ ...block, fields: fields.filter((_, i) => i !== idx) });
  }

  return (
    <div className="space-y-3">
      {fields.map((f, idx) => (
        <div key={idx} className="flex items-start gap-2 p-2 rounded border">
          <div className="flex-1 grid grid-cols-2 gap-2">
            <Input placeholder="Field name" value={f.name} onChange={(e) => updateField(idx, "name", e.target.value)} />
            <Input placeholder="Label" value={f.label} onChange={(e) => updateField(idx, "label", e.target.value)} />
            <select className="flex h-9 rounded-md border border-input bg-background px-3 py-1 text-sm" value={f.field_type} onChange={(e) => updateField(idx, "field_type", e.target.value)}>
              {FIELD_TYPES.map((ft) => <option key={ft} value={ft}>{ft}</option>)}
            </select>
            <div className="flex items-center gap-2">
              <label className="flex items-center gap-1.5 text-sm">
                <input type="checkbox" checked={f.required ?? false} onChange={(e) => updateField(idx, "required", e.target.checked)} />
                Required
              </label>
            </div>
            {f.field_type === "select" && (
              <Input className="col-span-2" placeholder="Options (comma separated)" value={f.options ?? ""} onChange={(e) => updateField(idx, "options", e.target.value)} />
            )}
          </div>
          <Button variant="ghost" size="icon-sm" onClick={() => removeField(idx)}>
            <Trash2 className="size-3" />
          </Button>
        </div>
      ))}
      <div className="flex gap-2">
        <Button variant="outline" size="sm" onClick={addField}>
          <Plus className="size-3 mr-1" />Field
        </Button>
        <Input placeholder="Submit button text" value={(block.submit_text as string) ?? ""} onChange={(e) => onChange({ ...block, submit_text: e.target.value })} className="max-w-xs" />
      </div>
    </div>
  );
}

function ColumnsEditor({
  block,
  onChange,
}: {
  block: Record<string, unknown>;
  onChange: (block: Record<string, unknown>) => void;
}) {
  const cols = (block.columns as { blocks: object[] }[]) ?? [{ blocks: [] }, { blocks: [] }];

  function updateColumn(colIdx: number, blocks: object[]) {
    const next = [...cols];
    next[colIdx] = { ...next[colIdx], blocks };
    onChange({ ...block, columns: next });
  }

  function addColumn() {
    onChange({ ...block, columns: [...cols, { blocks: [] }] });
  }

  function removeColumn(colIdx: number) {
    if (cols.length <= 1) return;
    onChange({ ...block, columns: cols.filter((_, i) => i !== colIdx) });
  }

  return (
    <div className="space-y-2">
      <div className="flex items-center gap-2 mb-2">
        <Label className="text-xs text-muted-foreground">{cols.length} columns</Label>
        <Button variant="outline" size="sm" onClick={addColumn} disabled={cols.length >= 6}>
          <Plus className="size-3 mr-1" />Column
        </Button>
      </div>
      <div className={`grid gap-2`} style={{ gridTemplateColumns: `repeat(${cols.length}, 1fr)` }}>
        {cols.map((col, colIdx) => (
          <div key={colIdx} className="rounded border p-2 space-y-2">
            <div className="flex items-center justify-between">
              <span className="text-xs font-medium text-muted-foreground">Col {colIdx + 1}</span>
              <Button variant="ghost" size="icon-sm" onClick={() => removeColumn(colIdx)} disabled={cols.length <= 1}>
                <Trash2 className="size-3" />
              </Button>
            </div>
            <MiniBlockList blocks={col.blocks} onChange={(b) => updateColumn(colIdx, b)} />
          </div>
        ))}
      </div>
    </div>
  );
}

function MiniBlockList({
  blocks,
  onChange,
}: {
  blocks: object[];
  onChange: (blocks: object[]) => void;
}) {
  const [showPicker, setShowPicker] = useState(false);

  function addBlock(type: string) {
    const defaults: Record<string, object> = {
      richtext: { type: "richtext", content: "" },
      image: { type: "image", url: "", alt: "" },
      video: { type: "video", url: "" },
      code: { type: "code", code: "", language: "" },
      html: { type: "html", content: "" },
      quote: { type: "quote", content: "" },
    };
    onChange([...blocks, defaults[type] ?? { type, content: "" }]);
    setShowPicker(false);
  }

  function updateBlock(idx: number, value: string) {
    const next = [...blocks] as Record<string, unknown>[];
    next[idx] = { ...next[idx], content: value };
    onChange(next);
  }

  function removeBlock(idx: number) {
    onChange(blocks.filter((_, i) => i !== idx));
  }

  function moveBlock(idx: number, dir: -1 | 1) {
    const target = idx + dir;
    if (target < 0 || target >= blocks.length) return;
    const next = [...blocks];
    [next[idx], next[target]] = [next[target], next[idx]];
    onChange(next);
  }

  const COL_BLOCK_TYPES = [
    { type: "richtext", label: "Text", icon: "📝" },
    { type: "image", label: "Image", icon: "📷" },
    { type: "video", label: "Video", icon: "🎬" },
    { type: "code", label: "Code", icon: "📋" },
    { type: "html", label: "HTML", icon: "🌐" },
    { type: "quote", label: "Quote", icon: "💬" },
  ];

  return (
    <div className="space-y-1">
      {blocks.map((b, idx) => {
        const rec = b as Record<string, unknown>;
        return (
          <div key={idx} className="flex items-center gap-1 text-xs bg-muted/50 rounded px-1.5 py-1">
            <span className="flex-1 truncate">{String(rec.type)}: {String((rec.content as string) ?? (rec.url as string) ?? "").slice(0, 20)}</span>
            <button type="button" className="hover:text-primary" onClick={() => moveBlock(idx, -1)}>↑</button>
            <button type="button" className="hover:text-primary" onClick={() => moveBlock(idx, 1)}>↓</button>
            <button type="button" className="hover:text-destructive" onClick={() => removeBlock(idx)}>✕</button>
          </div>
        );
      })}
      {showPicker ? (
        <div className="grid grid-cols-3 gap-1">
          {COL_BLOCK_TYPES.map((bt) => (
            <button key={bt.type} type="button" onClick={() => addBlock(bt.type)} className="text-xs rounded border px-1.5 py-1 hover:bg-muted">
              {bt.icon} {bt.label}
            </button>
          ))}
        </div>
      ) : (
        <button type="button" onClick={() => setShowPicker(true)} className="text-xs text-muted-foreground hover:text-primary w-full text-center py-1 border border-dashed rounded">
          + Add
        </button>
      )}
    </div>
  );
}
