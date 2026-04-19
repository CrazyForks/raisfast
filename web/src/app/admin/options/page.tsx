"use client";

import { useState } from "react";
import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import { Save, RotateCcw } from "lucide-react";
import { toast } from "sonner";

import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Checkbox } from "@/components/ui/checkbox";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { api, ApiError } from "@/lib/api";
import { useT } from "@/lib/i18n";

interface Validation {
  min?: number;
  max?: number;
  max_length?: number;
  values?: string[];
}

interface OptionEntry {
  key: string;
  value: unknown;
  type: string;
  label: string;
  description?: string;
  validation?: Validation;
  is_public: boolean;
}

interface OptionGroup {
  key: string;
  label: string;
  options: OptionEntry[];
}

const GROUP_LABELS: Record<string, string> = {
  general: "General",
  reading: "Reading",
  discussion: "Discussion",
  appearance: "Appearance",
};

function OptionField({
  option,
  value,
  onChange,
}: {
  option: OptionEntry;
  value: unknown;
  onChange: (v: unknown) => void;
}) {
  const desc = option.description ? (
    <p className="text-xs text-muted-foreground mt-1">{option.description}</p>
  ) : null;

  switch (option.type) {
    case "boolean":
      return (
        <div className="flex items-center justify-between rounded-lg border p-3">
          <div className="space-y-0.5">
            <Label className="text-base cursor-pointer" htmlFor={`opt-${option.key}`}>
              {option.label}
            </Label>
            {desc}
          </div>
          <Checkbox
            id={`opt-${option.key}`}
            checked={value === true || value === "true"}
            onCheckedChange={(checked) => onChange(checked === true)}
          />
        </div>
      );

    case "integer":
      return (
        <div className="space-y-2">
          <Label>{option.label}</Label>
          <Input
            type="number"
            value={String(value ?? "")}
            min={option.validation?.min}
            max={option.validation?.max}
            onChange={(e) => {
              const v = e.target.value;
              onChange(v === "" ? "" : Number(v));
            }}
          />
          {desc}
        </div>
      );

    case "select":
      return (
        <div className="space-y-2">
          <Label>{option.label}</Label>
          <Select value={String(value ?? "")} onValueChange={(v) => onChange(v)}>
            <SelectTrigger className="w-full">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              {option.validation?.values?.map((v) => (
                <SelectItem key={v} value={v}>
                  {v}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
          {desc}
        </div>
      );

    case "email":
      return (
        <div className="space-y-2">
          <Label>{option.label}</Label>
          <Input
            type="email"
            value={String(value ?? "")}
            onChange={(e) => onChange(e.target.value)}
          />
          {desc}
        </div>
      );

    case "url":
      return (
        <div className="space-y-2">
          <Label>{option.label}</Label>
          <Input
            type="url"
            value={String(value ?? "")}
            onChange={(e) => onChange(e.target.value)}
          />
          {desc}
        </div>
      );

    default:
      return (
        <div className="space-y-2">
          <Label>{option.label}</Label>
          <Input
            type="text"
            value={String(value ?? "")}
            maxLength={option.validation?.max_length}
            onChange={(e) => onChange(e.target.value)}
          />
          {desc}
        </div>
      );
  }
}

export default function OptionsPage() {
  const queryClient = useQueryClient();
  const { t } = useT();
  const [dirty, setDirty] = useState<Record<string, unknown>>({});

  const groupsQuery = useQuery({
    queryKey: ["options"],
    queryFn: () => api.get<OptionGroup[]>("/admin/options"),
  });

  const groups = groupsQuery.data ?? [];

  const saveMutation = useMutation({
    mutationFn: (options: Record<string, unknown>) =>
      api.put<OptionGroup[]>("/admin/options", { options }),
    onSuccess: (data) => {
      toast.success(t("options.saved"));
      setDirty({});
      queryClient.setQueryData(["options"], data);
    },
    onError: (err) => {
      toast.error(err instanceof ApiError ? err.message : t("options.failedToSave"));
    },
  });

  function getValue(option: OptionEntry): unknown {
    if (option.key in dirty) return dirty[option.key];
    return option.value;
  }

  function handleChange(key: string, value: unknown) {
    setDirty((prev) => ({ ...prev, [key]: value }));
  }

  function handleSave() {
    const updates: Record<string, unknown> = {};
    for (const group of groups) {
      for (const opt of group.options) {
        if (opt.key in dirty) {
          updates[opt.key] = dirty[opt.key];
        }
      }
    }
    if (Object.keys(updates).length === 0) {
      toast.info(t("options.noChanges"));
      return;
    }
    saveMutation.mutate(updates);
  }

  function handleReset() {
    setDirty({});
  }

  const hasChanges = Object.keys(dirty).length > 0;
  const firstGroup = groups[0]?.key ?? "general";

  return (
    <div className="space-y-6">
      <div className="flex items-center justify-between">
        <h1 className="text-2xl font-bold">{t("options.title")}</h1>
        <div className="flex items-center gap-2">
          {hasChanges && (
            <Button variant="outline" size="sm" onClick={handleReset}>
              <RotateCcw className="size-4 mr-1" />
              {t("options.reset")}
            </Button>
          )}
          <Button
            size="sm"
            onClick={handleSave}
            disabled={!hasChanges || saveMutation.isPending}
          >
            <Save className="size-4 mr-1" />
            {saveMutation.isPending ? t("common.saving") : t("options.saveChanges")}
          </Button>
        </div>
      </div>

      {groupsQuery.isLoading ? (
        <div className="space-y-4">
          {[1, 2, 3].map((i) => (
            <Card key={i}>
              <CardContent className="p-6">
                <div className="animate-pulse space-y-4">
                  <div className="h-4 bg-muted rounded w-32" />
                  <div className="h-10 bg-muted rounded" />
                </div>
              </CardContent>
            </Card>
          ))}
        </div>
      ) : groups.length === 0 ? (
        <p className="text-muted-foreground">{t("options.noOptions")}</p>
      ) : (
        <Tabs defaultValue={firstGroup}>
          <TabsList>
            {groups.map((g) => (
              <TabsTrigger key={g.key} value={g.key}>
                {GROUP_LABELS[g.key] ?? g.label}
              </TabsTrigger>
            ))}
          </TabsList>
          {groups.map((group) => (
            <TabsContent key={group.key} value={group.key} className="mt-4">
              <Card>
                <CardHeader>
                  <CardTitle className="text-base">
                    {GROUP_LABELS[group.key] ?? group.label}
                  </CardTitle>
                </CardHeader>
                <CardContent className="space-y-6">
                  {group.options.map((opt) => (
                    <OptionField
                      key={opt.key}
                      option={opt}
                      value={getValue(opt)}
                      onChange={(v) => handleChange(opt.key, v)}
                    />
                  ))}
                </CardContent>
              </Card>
            </TabsContent>
          ))}
        </Tabs>
      )}
    </div>
  );
}
