
import { useState, useEffect, useCallback, useMemo } from "react";
import { useRouter, useSearchParams } from "@/lib/navigation";
import { useQuery, useMutation } from "@tanstack/react-query";
import { toast } from "sonner";
import { useT } from "@/lib/i18n";
import {
  Type,
  FileText,
  Mail,
  Lock,
  Fingerprint,
  Hash,
  ArrowUpDown,
  DollarSign,
  TrendingUp,
  Calendar,
  CalendarClock,
  Clock,
  ToggleLeft,
  List,
  Braces,
  Image,
  Link,
  Plus,
  Trash2,
  GripVertical,
  ChevronUp,
  ChevronDown,
} from "lucide-react";

import { Button } from "@/components/ui/button";
import {
  Card,
  CardContent,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Textarea } from "@/components/ui/textarea";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogDescription,
} from "@/components/ui/dialog";
import { Separator } from "@/components/ui/separator";
import { Badge } from "@/components/ui/badge";
import { Checkbox } from "@/components/ui/checkbox";
import { client } from "@/lib/raisfast";
import { SDKError, type ContentTypeSchema as ContentTypeSchemaSDK, type FieldDef as FieldDefSDK } from "@raisfast/sdk";
import type { FieldSchema, ContentTypeSchema } from "@/components/admin/field-renderer";

type FieldType = FieldSchema["field_type"];

interface FieldDraft {
  name: string;
  field_type: FieldType;
  required: boolean;
  unique: boolean;
  private: boolean;
  immutable: boolean;
  label: string;
  description: string;
  default: string;
  max_length: number | null;
  min: number | null;
  max: number | null;
  enum_values: string[];
  relation: {
    relation_type: string;
    target: string;
    through?: string;
    foreign_key?: string;
  } | null;
  media_config: { accept: string[]; max_count: number } | null;
}

interface BaseConfig {
  name: string;
  singular: string;
  plural: string;
  table: string;
  description: string;
  draft_publish: boolean;
  timestamps: boolean;
  soft_delete: boolean;
  slug_field: string;
}

const FIELD_TYPE_CATEGORIES: {
  label: string;
  types: { type: FieldType; label: string; icon: React.ElementType }[];
}[] = [
  {
    label: "Text",
    types: [
      { type: "text", label: "Text", icon: Type },
      { type: "richtext", label: "Rich Text", icon: FileText },
      { type: "email", label: "Email", icon: Mail },
      { type: "password", label: "Password", icon: Lock },
      { type: "uid", label: "UID", icon: Fingerprint },
    ],
  },
  {
    label: "Number",
    types: [
      { type: "integer", label: "Integer", icon: Hash },
      { type: "bigint", label: "Big Int", icon: ArrowUpDown },
      { type: "decimal", label: "Decimal", icon: DollarSign },
      { type: "float", label: "Float", icon: TrendingUp },
    ],
  },
  {
    label: "Date",
    types: [
      { type: "date", label: "Date", icon: Calendar },
      { type: "datetime", label: "DateTime", icon: CalendarClock },
      { type: "time", label: "Time", icon: Clock },
    ],
  },
  {
    label: "Other",
    types: [
      { type: "boolean", label: "Boolean", icon: ToggleLeft },
      { type: "enum", label: "Enum", icon: List },
      { type: "json", label: "JSON", icon: Braces },
      { type: "media", label: "Media", icon: Image },
      { type: "relation", label: "Relation", icon: Link },
    ],
  },
];

const ALL_FIELD_TYPES = FIELD_TYPE_CATEGORIES.flatMap((c) => c.types);

const FIELD_TYPE_ICON_MAP: Record<FieldType, React.ElementType> = {} as never;
for (const ft of ALL_FIELD_TYPES) {
  FIELD_TYPE_ICON_MAP[ft.type] = ft.icon;
}

function emptyField(type: FieldType): FieldDraft {
  return {
    name: "",
    field_type: type,
    required: false,
    unique: false,
    private: false,
    immutable: false,
    label: "",
    description: "",
    default: "",
    max_length: type === "text" || type === "email" || type === "password" ? 255 : null,
    min: null,
    max: null,
    enum_values: type === "enum" ? [] : [],
    relation:
      type === "relation"
        ? { relation_type: "one_to_many", target: "" }
        : null,
    media_config:
      type === "media" ? { accept: [], max_count: 1 } : null,
  };
}

function fieldSchemaToDraft(f: FieldSchema): FieldDraft {
  return {
    name: f.name,
    field_type: f.field_type,
    required: f.required,
    unique: f.unique,
    private: f.private,
    immutable: f.immutable,
    label: f.label ?? "",
    description: f.description ?? "",
    default: f.default == null ? "" : String(f.default),
    max_length: f.max_length,
    min: f.min,
    max: f.max,
    enum_values: f.enum_values ?? [],
    relation: f.relation
      ? { relation_type: f.relation.relation_type, target: f.relation.target }
      : null,
    media_config: f.media_config
      ? { accept: f.media_config.accept, max_count: f.media_config.max_count }
      : null,
  };
}

function slugify(s: string): string {
  return s
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, "_")
    .replace(/^_+|_+$/g, "");
}

export default function ContentTypeBuilderPage() {
  const { t } = useT();
  const router = useRouter();
  const searchParams = useSearchParams();
  const editSingular = searchParams.get("edit");
  const isEditMode = !!editSingular;

  const [fields, setFields] = useState<FieldDraft[]>([]);
  const [selectedIdx, setSelectedIdx] = useState<number | null>(null);
  const [pickerOpen, setPickerOpen] = useState(false);

  const [base, setBase] = useState<BaseConfig>({
    name: "",
    singular: "",
    plural: "",
    table: "",
    description: "",
    draft_publish: true,
    timestamps: true,
    soft_delete: false,
    slug_field: "",
  });

  const editQuery = useQuery({
    queryKey: ["content-type", editSingular],
    queryFn: () =>
      client.send<ContentTypeSchema>(`/admin/content-types/${editSingular}`),
    enabled: isEditMode,
  });

  useEffect(() => {
    if (editQuery.data) {
      const s = editQuery.data;
      setBase({
        name: s.name,
        singular: s.singular,
        plural: s.plural,
        table: s.table,
        description: s.description ?? "",
        draft_publish: s.draft_publish,
        timestamps: s.timestamps,
        soft_delete: s.soft_delete,
        slug_field: s.slug_field ?? "",
      });
      setFields(s.fields.map(fieldSchemaToDraft));
    }
  }, [editQuery.data]);

  const saveMutation = useMutation({
    mutationFn: () => {
      const fieldPayloads = fields.map((f) => {
        const baseField: Record<string, unknown> = {
          name: f.name,
          field_type: f.field_type,
          required: f.required,
          unique: f.unique,
          private: f.private,
          immutable: f.immutable,
          label: f.label || null,
          description: f.description || null,
          default: f.default || null,
        };
        if (f.field_type === "text" || f.field_type === "email" || f.field_type === "password") {
          baseField.max_length = f.max_length;
        }
        if (
          f.field_type === "integer" ||
          f.field_type === "bigint" ||
          f.field_type === "decimal" ||
          f.field_type === "float"
        ) {
          baseField.min = f.min;
          baseField.max = f.max;
        }
        if (f.field_type === "enum") {
          baseField.enum_values = f.enum_values;
        }
        if (f.field_type === "relation") {
          baseField.relation = f.relation;
        }
        if (f.field_type === "media") {
          baseField.media_config = f.media_config;
        }
        return baseField;
      });

      if (isEditMode) {
        return client.admin.contentTypes.update(editSingular, {
          name: base.name,
          description: base.description,
          draft_publish: base.draft_publish,
          timestamps: base.timestamps,
          soft_delete: base.soft_delete,
          slug_field: base.slug_field || null,
          fields: fieldPayloads,
        } as unknown as Partial<ContentTypeSchemaSDK>);
      }

      return client.admin.contentTypes.create({
        name: base.name,
        singular: base.singular,
        plural: base.plural,
        table: base.table,
        description: base.description,
        draft_publish: base.draft_publish,
        timestamps: base.timestamps,
        soft_delete: base.soft_delete,
        slug_field: base.slug_field || null,
        fields: fieldPayloads,
      } as unknown as ContentTypeSchemaSDK);
    },
    onSuccess: () => {
      toast.success(
        isEditMode ? t("builder.contentTypeUpdated") : t("builder.contentTypeCreated"),
      );
      router.push("/content-types");
    },
    onError: (err) => {
      if (err instanceof SDKError) {
        toast.error(err.message);
      } else {
        toast.error(t("builder.failedToSave"));
      }
    },
  });

  const updateField = useCallback(
    (idx: number, patch: Partial<FieldDraft>) => {
      setFields((prev) =>
        prev.map((f, i) => (i === idx ? { ...f, ...patch } : f)),
      );
    },
    [],
  );

  const removeField = useCallback((idx: number) => {
    setFields((prev) => prev.filter((_, i) => i !== idx));
    setSelectedIdx(null);
  }, []);

  const moveField = useCallback((idx: number, direction: -1 | 1) => {
    setFields((prev) => {
      const target = idx + direction;
      if (target < 0 || target >= prev.length) return prev;
      const next = [...prev];
      [next[idx], next[target]] = [next[target], next[idx]];
      if (selectedIdx === idx) setSelectedIdx(target);
      else if (selectedIdx === target) setSelectedIdx(idx);
      return next;
    });
  }, [selectedIdx]);

  const addField = useCallback((type: FieldType) => {
    const f = emptyField(type);
    setFields((prev) => {
      const next = [...prev, f];
      setSelectedIdx(next.length - 1);
      return next;
    });
    setPickerOpen(false);
  }, []);

  useEffect(() => {
    if (selectedIdx !== null && selectedIdx >= fields.length) {
      setSelectedIdx(fields.length > 0 ? fields.length - 1 : null);
    }
  }, [fields.length, selectedIdx]);

  const textFields = useMemo(
    () => fields.filter((f) => f.field_type === "text" || f.field_type === "uid"),
    [fields],
  );

  function handleNameChange(name: string) {
    const singular = slugify(name);
    const plural = singular.endsWith("s") ? singular : singular + "s";
    const table = plural;
    setBase((prev) => ({
      ...prev,
      name,
      singular: prev.singular || singular,
      plural: prev.plural || plural,
      table: prev.table || table,
    }));
  }

  const selectedField =
    selectedIdx !== null && selectedIdx < fields.length
      ? fields[selectedIdx]
      : null;

  if (isEditMode && editQuery.isLoading) {
    return (
      <div className="flex items-center justify-center py-20">
        <div className="size-8 animate-spin rounded-full border-2 border-muted border-t-transparent" />
      </div>
    );
  }

  return (
    <div className="flex flex-col h-[calc(100vh-6rem)]">
      <div className="flex items-center gap-4 mb-4">
        <Button
          variant="outline"
          size="sm"
          onClick={() => router.push("/content-types")}
        >
          &larr; {t("common.back")}
        </Button>
        <h1 className="text-2xl font-bold">
          {isEditMode ? `${t("builder.editContentType")}: ${base.name}` : t("builder.title")}
        </h1>
      </div>

      <div className="flex flex-1 gap-4 min-h-0">
        <div className="w-1/3 flex flex-col gap-2 overflow-y-auto">
          <Card
            className={`cursor-pointer transition-colors ${
              selectedIdx === null ? "ring-2 ring-primary" : ""
            }`}
            onClick={() => setSelectedIdx(null)}
          >
            <CardHeader className="py-3 px-4">
              <CardTitle className="text-sm flex items-center gap-2">
                <FileText className="size-4" />
                {base.name || t("builder.untitled")}
              </CardTitle>
            </CardHeader>
            <CardContent className="py-2 px-4">
              <p className="text-xs text-muted-foreground">
                {t("builder.fieldCount", { count: fields.length })}
              </p>
            </CardContent>
          </Card>

          <Separator />

          <div className="flex flex-col gap-1">
            {fields.map((f, idx) => {
              const Icon = FIELD_TYPE_ICON_MAP[f.field_type] ?? Type;
              const isSelected = idx === selectedIdx;
              return (
                <button
                  key={idx}
                  type="button"
                  onClick={() => setSelectedIdx(idx)}
                  className={`flex items-center gap-2 rounded-lg px-3 py-2 text-left text-sm transition-colors w-full ${
                    isSelected
                      ? "bg-primary/10 text-primary"
                      : "hover:bg-muted"
                  }`}
                >
                  <div className="flex flex-col gap-0">
                    <button
                      type="button"
                      className="p-0 hover:text-foreground disabled:opacity-30"
                      disabled={idx === 0}
                      onClick={(e) => { e.stopPropagation(); moveField(idx, -1); }}
                    >
                      <ChevronUp className="size-3" />
                    </button>
                    <button
                      type="button"
                      className="p-0 hover:text-foreground disabled:opacity-30"
                      disabled={idx === fields.length - 1}
                      onClick={(e) => { e.stopPropagation(); moveField(idx, 1); }}
                    >
                      <ChevronDown className="size-3" />
                    </button>
                  </div>
                  <Icon className="size-4 shrink-0" />
                  <span className="flex-1 truncate">
                    {f.name || t("builder.newField")}
                  </span>
                  <Badge variant="secondary" className="text-[10px] px-1.5">
                    {f.field_type}
                  </Badge>
                </button>
              );
            })}
          </div>

          <Button
            variant="outline"
            className="w-full mt-2"
            onClick={() => setPickerOpen(true)}
          >
            <Plus className="size-4" />
            {t("builder.addAnotherField")}
          </Button>
        </div>

        <div className="w-2/3 overflow-y-auto">
          {selectedField ? (
            <FieldConfigPanel
              field={selectedField}
              onUpdate={(patch) =>
                selectedIdx !== null && updateField(selectedIdx, patch)
              }
              onRemove={() =>
                selectedIdx !== null && removeField(selectedIdx)
              }
            />
          ) : (
            <BaseConfigPanel
              base={base}
              onChange={setBase}
              textFields={textFields}
            />
          )}
        </div>
      </div>

      <Separator className="my-4" />

      <div className="flex justify-end gap-2">
        <Button
          variant="outline"
          onClick={() => router.push("/content-types")}
        >
          {t("common.cancel")}
        </Button>
        <Button
          onClick={() => saveMutation.mutate()}
          disabled={saveMutation.isPending}
        >
          {saveMutation.isPending ? t("builder.saving") : t("builder.save")}
        </Button>
      </div>

      <Dialog open={pickerOpen} onOpenChange={setPickerOpen}>
        <DialogContent className="sm:max-w-2xl">
          <DialogHeader>
            <DialogTitle>{t("builder.addAField")}</DialogTitle>
            <DialogDescription>
              {t("builder.chooseFieldType")}
            </DialogDescription>
          </DialogHeader>
          <div className="space-y-4 mt-2">
            {FIELD_TYPE_CATEGORIES.map((cat) => (
              <div key={cat.label}>
                <p className="text-xs font-medium text-muted-foreground mb-2 uppercase tracking-wider">
                  {cat.label}
                </p>
                <div className="grid grid-cols-3 gap-2">
                  {cat.types.map((ft) => {
                    const Icon = ft.icon;
                    return (
                      <button
                        key={ft.type}
                        type="button"
                        onClick={() => addField(ft.type)}
                        className="flex items-center gap-2 rounded-lg border p-3 text-sm hover:bg-muted transition-colors"
                      >
                        <Icon className="size-4 text-muted-foreground" />
                        <span>{ft.label}</span>
                      </button>
                    );
                  })}
                </div>
              </div>
            ))}
          </div>
        </DialogContent>
      </Dialog>
    </div>
  );
}

function BaseConfigPanel({
  base,
  onChange,
  textFields,
}: {
  base: BaseConfig;
  onChange: (b: BaseConfig) => void;
  textFields: FieldDraft[];
}) {
  const { t } = useT();

  function upd(patch: Partial<BaseConfig>) {
    onChange({ ...base, ...patch });
  }

  return (
    <Card>
      <CardHeader>
        <CardTitle className="text-base">{t("builder.baseConfig")}</CardTitle>
      </CardHeader>
      <CardContent className="space-y-4">
        <div className="space-y-2">
          <Label>{t("common.name")}</Label>
          <Input
            value={base.name}
            onChange={(e) => {
              const name = e.target.value;
              const singular = slugify(name);
              const plural = singular.endsWith("s")
                ? singular
                : singular + "s";
              onChange({
                ...base,
                name,
                singular,
                plural,
                table: plural,
              });
            }}
            placeholder="e.g. Product"
          />
        </div>

        <div className="grid grid-cols-3 gap-4">
          <div className="space-y-2">
            <Label>{t("builder.singularName")}</Label>
            <Input
              value={base.singular}
              onChange={(e) => upd({ singular: e.target.value })}
              placeholder="product"
            />
          </div>
          <div className="space-y-2">
            <Label>{t("builder.pluralName")}</Label>
            <Input
              value={base.plural}
              onChange={(e) => upd({ plural: e.target.value })}
              placeholder="products"
            />
          </div>
          <div className="space-y-2">
            <Label>{t("builder.tableName")}</Label>
            <Input
              value={base.table}
              onChange={(e) => upd({ table: e.target.value })}
              placeholder="products"
            />
          </div>
        </div>

        <div className="space-y-2">
          <Label>{t("common.description")}</Label>
          <Textarea
            value={base.description}
            onChange={(e) => upd({ description: e.target.value })}
            placeholder={t("builder.describeContentType")}
            rows={3}
          />
        </div>

        <Separator />

        <div className="grid grid-cols-3 gap-4">
          <label className="flex items-center gap-2 cursor-pointer">
            <Checkbox
              checked={base.draft_publish}
              onCheckedChange={(v) => upd({ draft_publish: v === true })}
            />
            <span className="text-sm">{t("builder.draftPublish")}</span>
          </label>
          <label className="flex items-center gap-2 cursor-pointer">
            <Checkbox
              checked={base.timestamps}
              onCheckedChange={(v) => upd({ timestamps: v === true })}
            />
            <span className="text-sm">{t("builder.timestamps")}</span>
          </label>
          <label className="flex items-center gap-2 cursor-pointer">
            <Checkbox
              checked={base.soft_delete}
              onCheckedChange={(v) => upd({ soft_delete: v === true })}
            />
            <span className="text-sm">{t("builder.softDelete")}</span>
          </label>
        </div>

        {textFields.length > 0 && (
          <div className="space-y-2">
            <Label>{t("builder.slugField")}</Label>
            <Select
              value={base.slug_field || "__none__"}
              onValueChange={(v) =>
                upd({ slug_field: v === "__none__" ? "" : v ?? "" })
              }
            >
              <SelectTrigger className="w-full">
                <SelectValue placeholder={t("common.none")} />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="__none__">{t("common.none")}</SelectItem>
                {textFields.map((f) => (
                  <SelectItem key={f.name} value={f.name}>
                    {f.name || "unnamed"}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </div>
        )}
      </CardContent>
    </Card>
  );
}

function FieldConfigPanel({
  field,
  onUpdate,
  onRemove,
}: {
  field: FieldDraft;
  onUpdate: (patch: Partial<FieldDraft>) => void;
  onRemove: () => void;
}) {
  const { t } = useT();

  return (
    <Card>
      <CardHeader>
        <CardTitle className="text-base flex items-center gap-2">
          {(() => {
            const Icon = FIELD_TYPE_ICON_MAP[field.field_type] ?? Type;
            return <Icon className="size-4" />;
          })()}
          {t("builder.fieldConfiguration")}
          <Badge variant="secondary" className="ml-auto">
            {field.field_type}
          </Badge>
        </CardTitle>
      </CardHeader>
      <CardContent className="space-y-4">
        <div className="grid grid-cols-2 gap-4">
          <div className="space-y-2">
            <Label>{t("builder.fieldName")}</Label>
            <Input
              value={field.name}
              onChange={(e) => onUpdate({ name: slugify(e.target.value) })}
              placeholder="e.g. title"
            />
          </div>
          <div className="space-y-2">
            <Label>{t("builder.fieldTypeLabel")}</Label>
            <Input value={field.field_type} disabled />
          </div>
        </div>

        <div className="grid grid-cols-2 gap-4">
          <div className="space-y-2">
            <Label>{t("builder.label")}</Label>
            <Input
              value={field.label}
              onChange={(e) => onUpdate({ label: e.target.value })}
              placeholder={t("builder.displayLabel")}
            />
          </div>
          <div className="space-y-2">
            <Label>{t("common.description")}</Label>
            <Input
              value={field.description}
              onChange={(e) => onUpdate({ description: e.target.value })}
              placeholder={t("builder.helpText")}
            />
          </div>
        </div>

        <div className="space-y-2">
          <Label>{t("builder.defaultValue")}</Label>
          <Input
            value={field.default}
            onChange={(e) => onUpdate({ default: e.target.value })}
            placeholder="Default value"
          />
        </div>

        <Separator />

        <div className="grid grid-cols-2 gap-4">
          <label className="flex items-center gap-2 cursor-pointer">
            <Checkbox
              checked={field.required}
              onCheckedChange={(v) => onUpdate({ required: v === true })}
            />
            <span className="text-sm">{t("builder.required")}</span>
          </label>
          <label className="flex items-center gap-2 cursor-pointer">
            <Checkbox
              checked={field.unique}
              onCheckedChange={(v) => onUpdate({ unique: v === true })}
            />
            <span className="text-sm">{t("builder.unique")}</span>
          </label>
          <label className="flex items-center gap-2 cursor-pointer">
            <Checkbox
              checked={field.private}
              onCheckedChange={(v) => onUpdate({ private: v === true })}
            />
            <span className="text-sm">{t("builder.privateField")}</span>
          </label>
          <label className="flex items-center gap-2 cursor-pointer">
            <Checkbox
              checked={field.immutable}
              onCheckedChange={(v) => onUpdate({ immutable: v === true })}
            />
            <span className="text-sm">{t("builder.immutable")}</span>
          </label>
        </div>

        <Separator />

        <TypeSpecificOptions field={field} onUpdate={onUpdate} />

        <Separator />

        <Button
          variant="destructive"
          size="sm"
          onClick={() => {
            if (confirm(t("builder.removeFieldConfirm"))) onRemove();
          }}
        >
          <Trash2 className="size-4" />
          {t("builder.removeField")}
        </Button>
      </CardContent>
    </Card>
  );
}

function TypeSpecificOptions({
  field,
  onUpdate,
}: {
  field: FieldDraft;
  onUpdate: (patch: Partial<FieldDraft>) => void;
}) {
  const { t } = useT();

  switch (field.field_type) {
    case "text":
    case "email":
    case "password":
      return (
        <div className="space-y-2">
          <Label>{t("builder.maxLength")}</Label>
          <Input
            type="number"
            value={field.max_length ?? ""}
            onChange={(e) => {
              const v = e.target.value;
              onUpdate({ max_length: v === "" ? null : Number(v) });
            }}
            placeholder="255"
          />
        </div>
      );

    case "integer":
    case "bigint":
    case "decimal":
    case "float":
      return (
        <div className="grid grid-cols-2 gap-4">
          <div className="space-y-2">
            <Label>{t("builder.minValue")}</Label>
            <Input
              type="number"
              value={field.min ?? ""}
              onChange={(e) => {
                const v = e.target.value;
                onUpdate({ min: v === "" ? null : Number(v) });
              }}
              placeholder="Min value"
            />
          </div>
          <div className="space-y-2">
            <Label>{t("builder.maxValue")}</Label>
            <Input
              type="number"
              value={field.max ?? ""}
              onChange={(e) => {
                const v = e.target.value;
                onUpdate({ max: v === "" ? null : Number(v) });
              }}
              placeholder="Max value"
            />
          </div>
        </div>
      );

    case "enum":
      return (
        <div className="space-y-2">
          <Label>{t("builder.valuesComma")}</Label>
          <Input
            value={(field.enum_values ?? []).join(", ")}
            onChange={(e) => {
              const vals = e.target.value
                .split(",")
                .map((s) => s.trim())
                .filter(Boolean);
              onUpdate({ enum_values: vals });
            }}
            placeholder="draft, published, archived"
          />
        </div>
      );

    case "relation":
      return (
        <div className="grid grid-cols-2 gap-4">
          <div className="space-y-2">
            <Label>{t("builder.target")}</Label>
            <Input
              value={field.relation?.target ?? ""}
              onChange={(e) =>
                onUpdate({
                  relation: {
                    relation_type: field.relation?.relation_type ?? "one_to_many",
                    target: e.target.value,
                  },
                })
              }
              placeholder="e.g. posts"
            />
          </div>
          <div className="space-y-2">
            <Label>{t("builder.relationType")}</Label>
            <Select
              value={field.relation?.relation_type ?? "one_to_many"}
              onValueChange={(v) =>
                onUpdate({
                  relation: {
                    relation_type: v ?? "one_to_many",
                    target: field.relation?.target ?? "",
                  },
                })
              }
            >
              <SelectTrigger className="w-full">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="one_to_one">One to One</SelectItem>
                <SelectItem value="one_to_many">One to Many</SelectItem>
                <SelectItem value="many_to_one">Many to One</SelectItem>
                <SelectItem value="many_to_many">Many to Many</SelectItem>
              </SelectContent>
            </Select>
          </div>
        </div>
      );

    case "media":
      return (
        <div className="grid grid-cols-2 gap-4">
          <div className="space-y-2">
            <Label>{t("builder.acceptedTypes")}</Label>
            <Input
              value={(field.media_config?.accept ?? []).join(", ")}
              onChange={(e) => {
                const vals = e.target.value
                  .split(",")
                  .map((s) => s.trim())
                  .filter(Boolean);
                onUpdate({
                  media_config: {
                    accept: vals,
                    max_count: field.media_config?.max_count ?? 1,
                  },
                });
              }}
              placeholder="image/*, .pdf"
            />
          </div>
          <div className="space-y-2">
            <Label>{t("builder.maxCount")}</Label>
            <Input
              type="number"
              value={field.media_config?.max_count ?? 1}
              onChange={(e) => {
                const v = e.target.value;
                onUpdate({
                  media_config: {
                    accept: field.media_config?.accept ?? [],
                    max_count: v === "" ? 1 : Number(v),
                  },
                });
              }}
              placeholder="1"
            />
          </div>
        </div>
      );

    case "richtext":
    case "boolean":
    case "date":
    case "datetime":
    case "time":
    case "uid":
    case "json":
      return (
        <p className="text-sm text-muted-foreground">
          {t("builder.noAdditionalOptions")}
        </p>
      );

    default:
      return null;
  }
}
