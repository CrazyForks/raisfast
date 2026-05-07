
import type { ReactNode } from "react";
import { Check, X } from "lucide-react";
import { Input } from "@/components/ui/input";
import { Textarea } from "@/components/ui/textarea";
import { Label } from "@/components/ui/label";
import { Badge } from "@/components/ui/badge";
import { MarkdownEditor } from "@/components/common/markdown-editor";
import { useT } from "@/lib/i18n";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";

export interface FieldSchema {
  name: string;
  field_type:
    | "text"
    | "richtext"
    | "integer"
    | "bigint"
    | "decimal"
    | "float"
    | "boolean"
    | "date"
    | "datetime"
    | "time"
    | "email"
    | "password"
    | "enum"
    | "uid"
    | "json"
    | "media"
    | "relation";
  required: boolean;
  unique: boolean;
  default: unknown;
  private: boolean;
  immutable: boolean;
  label: string | null;
  description: string | null;
  max_length: number | null;
  min: number | null;
  max: number | null;
  pattern: string | null;
  relation: {
    relation_type: string;
    target: string;
    through?: string;
    foreign_key?: string;
  } | null;
  media_config: { accept: string[]; max_count: number } | null;
  enum_values: string[] | null;
}

export interface ListViewConfig {
  default_sort: string;
  columns: string[];
}

export interface ContentTypeSchema {
  name: string;
  singular: string;
  plural: string;
  table: string;
  description: string;
  draft_publish: boolean;
  slug_field: string | null;
  timestamps: boolean;
  soft_delete: boolean;
  fields: FieldSchema[];
  indexes: { name: string; fields: string[]; unique: boolean }[];
  list_view: ListViewConfig | null;
}

export interface CmsItem {
  id: string;
  [key: string]: unknown;
}

export interface PaginatedCmsResponse {
  items: CmsItem[];
  total: number;
  page: number;
  page_size: number;
}

export function getFieldLabel(field: FieldSchema): string {
  return field.label ?? field.name;
}

export function getDisplayColumns(schema: ContentTypeSchema): string[] {
  if (schema.list_view?.columns && schema.list_view.columns.length > 0) {
    return schema.list_view.columns;
  }
  return schema.fields
    .filter((f) => !f.private && f.name !== "id")
    .slice(0, 5)
    .map((f) => f.name);
}

export function parseSort(sortStr: string): {
  field: string;
  direction: "asc" | "desc";
} {
  const parts = sortStr.split(",");
  const first = parts[0] ?? "created_at:desc";
  const [field, direction] = first.split(":");
  return {
    field: field ?? "created_at",
    direction: direction === "asc" ? "asc" : "desc",
  };
}

export function getFieldByName(
  schema: ContentTypeSchema,
  name: string,
): FieldSchema | null {
  return schema.fields.find((f) => f.name === name) ?? null;
}

function truncate(str: string, max: number): string {
  if (str.length <= max) return str;
  return str.slice(0, max) + "\u2026";
}

function stripHtml(html: string): string {
  return html.replace(/<[^>]*>/g, "");
}

function getEnumBadgeVariant(
  value: string,
): "default" | "secondary" | "destructive" | "outline" {
  const map: Record<string, "default" | "secondary" | "destructive" | "outline"> = {
    draft: "secondary",
    pending: "secondary",
    published: "default",
    active: "default",
    archived: "outline",
    inactive: "outline",
    deleted: "destructive",
    error: "destructive",
  };
  return map[value] ?? "secondary";
}

interface FieldRendererProps {
  field: FieldSchema;
  value: unknown;
  onChange: (value: unknown) => void;
  error?: string;
}

export function FieldRenderer({
  field,
  value,
  onChange,
  error,
}: FieldRendererProps) {
  const label = getFieldLabel(field);
  const { t } = useT();
  const strValue = value == null ? "" : String(value);

  function handleNumberChange(e: React.ChangeEvent<HTMLInputElement>) {
    const v = e.target.value;
    if (v === "") {
      onChange(null);
    } else {
      const num = Number(v);
      onChange(isNaN(num) ? null : num);
    }
  }

  let input: ReactNode;

  switch (field.field_type) {
    case "text":
    case "email":
    case "password":
    case "uid":
    case "media":
      input = (
        <Input
          type={
            field.field_type === "email"
              ? "email"
              : field.field_type === "password"
                ? "password"
                : "text"
          }
          value={strValue}
          onChange={(e) => onChange(e.target.value)}
          disabled={field.field_type === "uid"}
          maxLength={field.max_length ?? undefined}
          placeholder={field.field_type === "media" ? t("field.mediaUrl") : undefined}
        />
      );
      break;

    case "richtext":
      input = (
        <MarkdownEditor
          value={strValue}
          onChange={onChange}
          placeholder={t("field.startWriting")}
        />
      );
      break;

    case "integer":
    case "bigint":
      input = (
        <Input
          type="number"
          value={value == null ? "" : String(value)}
          onChange={handleNumberChange}
          min={field.min ?? undefined}
          max={field.max ?? undefined}
          step={1}
        />
      );
      break;

    case "decimal":
    case "float":
      input = (
        <Input
          type="number"
          value={value == null ? "" : String(value)}
          onChange={handleNumberChange}
          min={field.min ?? undefined}
          max={field.max ?? undefined}
          step={0.01}
        />
      );
      break;

    case "boolean":
      input = (
        <label className="flex items-center gap-2 cursor-pointer">
          <button
            type="button"
            role="switch"
            aria-checked={value === true}
            onClick={() => onChange(!value)}
            className={`relative inline-flex h-6 w-11 shrink-0 cursor-pointer rounded-full border-2 border-transparent transition-colors ${
              value ? "bg-primary" : "bg-input"
            }`}
          >
            <span
              className={`pointer-events-none block size-5 rounded-full bg-background shadow-sm ring-0 transition-transform ${
                value ? "translate-x-5" : "translate-x-0"
              }`}
            />
          </button>
          <span className="text-sm">{value ? t("field.yes") : t("field.no")}</span>
        </label>
      );
      break;

    case "date":
      input = (
        <Input
          type="date"
          value={strValue ? strValue.split("T")[0] : ""}
          onChange={(e) => onChange(e.target.value)}
        />
      );
      break;

    case "datetime":
      input = (
        <Input
          type="datetime-local"
          value={
            strValue
              ? new Date(strValue).toISOString().slice(0, 16)
              : ""
          }
          onChange={(e) => onChange(e.target.value)}
        />
      );
      break;

    case "time":
      input = (
        <Input
          type="time"
          value={strValue}
          onChange={(e) => onChange(e.target.value)}
        />
      );
      break;

    case "enum":
      input = (
        <Select
          value={strValue || undefined}
          onValueChange={(val) => {
            if (val) onChange(val);
          }}
        >
          <SelectTrigger className="w-full">
            <SelectValue placeholder={`Select ${label}`} />
          </SelectTrigger>
          <SelectContent>
            {(field.enum_values ?? []).map((opt) => (
              <SelectItem key={opt} value={opt}>
                {opt}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
      );
      break;

    case "json":
      input = (
        <Textarea
          value={strValue}
          onChange={(e) => onChange(e.target.value)}
          rows={6}
          className="font-mono text-sm"
          placeholder="{}"
        />
      );
      break;

    case "relation":
      input = (
        <Input
          type="text"
          value={strValue}
          onChange={(e) => onChange(e.target.value)}
          placeholder={
            field.relation?.foreign_key
              ? `${field.relation.foreign_key} value`
              : t("field.relatedItemId")
          }
        />
      );
      break;

    default:
      input = (
        <Input
          type="text"
          value={strValue}
          onChange={(e) => onChange(e.target.value)}
        />
      );
  }

  return (
    <div className="space-y-2">
      <Label>
        {label}
        {field.required && <span className="text-red-500 ml-1">*</span>}
      </Label>
      {input}
      {field.description && (
        <p className="text-xs text-muted-foreground">{field.description}</p>
      )}
      {error && <p className="text-sm text-red-500">{error}</p>}
    </div>
  );
}

interface FieldCellProps {
  field: FieldSchema | null;
  value: unknown;
  columnName: string;
}

export function FieldCell({ field, value, columnName }: FieldCellProps) {
  if (value == null || value === "") {
    return <span className="text-muted-foreground">\u2014</span>;
  }

  if (columnName === "status") {
    return (
      <Badge variant={getEnumBadgeVariant(String(value))}>
        {String(value)}
      </Badge>
    );
  }

  if (columnName === "id") {
    return (
      <span className="font-mono text-xs">
        {truncate(String(value), 8)}
      </span>
    );
  }

  if (
    columnName === "created_at" ||
    columnName === "updated_at" ||
    columnName === "published_at"
  ) {
    try {
      return (
        <span className="text-sm">
          {new Date(String(value)).toLocaleString()}
        </span>
      );
    } catch {
      return <span className="text-sm">{String(value)}</span>;
    }
  }

  if (!field) {
    return (
      <span className="text-sm truncate max-w-[200px] block">
        {truncate(String(value), 50)}
      </span>
    );
  }

  switch (field.field_type) {
    case "text":
    case "email":
    case "uid":
    case "media":
    case "relation":
      return (
        <span className="text-sm truncate max-w-[200px] block">
          {truncate(String(value), 50)}
        </span>
      );

    case "richtext":
      return (
        <span className="text-sm truncate max-w-[200px] block">
          {truncate(stripHtml(String(value)), 50)}
        </span>
      );

    case "boolean":
      return value ? (
        <Check className="size-4 text-green-600" />
      ) : (
        <X className="size-4 text-muted-foreground" />
      );

    case "integer":
    case "bigint":
    case "decimal":
    case "float":
      return <span className="text-sm font-mono">{String(value)}</span>;

    case "date":
      try {
        return (
          <span className="text-sm">
            {new Date(String(value)).toLocaleDateString()}
          </span>
        );
      } catch {
        return <span className="text-sm">{String(value)}</span>;
      }

    case "datetime":
      try {
        return (
          <span className="text-sm">
            {new Date(String(value)).toLocaleString()}
          </span>
        );
      } catch {
        return <span className="text-sm">{String(value)}</span>;
      }

    case "time":
      return <span className="text-sm">{String(value)}</span>;

    case "enum":
      return (
        <Badge variant={getEnumBadgeVariant(String(value))}>
          {String(value)}
        </Badge>
      );

    case "password":
      return <span className="text-sm text-muted-foreground">\u2022\u2022\u2022\u2022\u2022\u2022</span>;

    case "json":
      return (
        <span className="text-sm font-mono truncate max-w-[200px] block">
          {truncate(String(value), 50)}
        </span>
      );

    default:
      return (
        <span className="text-sm truncate max-w-[200px] block">
          {truncate(String(value), 50)}
        </span>
      );
  }
}
