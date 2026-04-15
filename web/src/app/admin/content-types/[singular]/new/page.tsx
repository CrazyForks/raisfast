"use client";

import { use, useState } from "react";
import { useRouter } from "next/navigation";
import { useQuery, useMutation } from "@tanstack/react-query";
import { toast } from "sonner";
import Link from "next/link";

import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import { Label } from "@/components/ui/label";
import { Skeleton } from "@/components/ui/skeleton";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { api, ApiError } from "@/lib/api";
import {
  type ContentTypeSchema,
  type FieldSchema,
  FieldRenderer,
  getFieldLabel,
} from "@/components/admin/field-renderer";

function getFormFields(schema: ContentTypeSchema): FieldSchema[] {
  return schema.fields.filter((f) => {
    if (f.private) return false;
    if (f.name === "id") return false;
    if (f.name === "status") return false;
    if (f.name === "created_at" || f.name === "updated_at") return false;
    return true;
  });
}

function getDefaults(schema: ContentTypeSchema): Record<string, unknown> {
  const defaults: Record<string, unknown> = {};
  for (const field of schema.fields) {
    if (field.default !== undefined && field.default !== null) {
      defaults[field.name] = field.default;
    }
  }
  if (schema.draft_publish) {
    defaults["status"] = "draft";
  }
  return defaults;
}

export default function NewCmsItemPage({
  params,
}: {
  params: Promise<{ singular: string }>;
}) {
  const { singular } = use(params);
  const router = useRouter();

  const schemaQuery = useQuery({
    queryKey: ["content-type", singular],
    queryFn: () =>
      api.get<ContentTypeSchema>(`/admin/content-types/${singular}`),
  });

  const schema = schemaQuery.data;

  const [formData, setFormData] = useState<Record<string, unknown>>({});
  const [errors, setErrors] = useState<Record<string, string>>({});
  const [initialized, setInitialized] = useState(false);

  if (schema && !initialized) {
    setFormData(getDefaults(schema));
    setInitialized(true);
  }

  const createMutation = useMutation({
    mutationFn: (data: Record<string, unknown>) =>
      api.post(`/cms/${schema!.plural}`, data),
    onSuccess: () => {
      toast.success(`${schema!.name} created`);
      router.push(`/admin/content-types/${singular}`);
    },
    onError: (err) => {
      if (err instanceof ApiError) {
        toast.error(err.message);
      } else {
        toast.error(`Failed to create ${schema!.name}`);
      }
    },
  });

  function handleChange(fieldName: string, value: unknown) {
    setFormData((prev) => ({ ...prev, [fieldName]: value }));
    setErrors((prev) => {
      const next = { ...prev };
      delete next[fieldName];
      return next;
    });
  }

  function validate(): boolean {
    if (!schema) return false;
    const newErrors: Record<string, string> = {};
    for (const field of getFormFields(schema)) {
      if (field.required && (formData[field.name] == null || formData[field.name] === "")) {
        newErrors[field.name] = `${getFieldLabel(field)} is required`;
      }
      if (
        field.field_type === "json" &&
        formData[field.name] &&
        typeof formData[field.name] === "string"
      ) {
        try {
          JSON.parse(formData[field.name] as string);
        } catch {
          newErrors[field.name] = "Invalid JSON";
        }
      }
    }
    setErrors(newErrors);
    return Object.keys(newErrors).length === 0;
  }

  function handleSubmit(e: React.FormEvent) {
    e.preventDefault();
    if (!validate()) return;
    createMutation.mutate(formData);
  }

  const formFields = schema ? getFormFields(schema) : [];

  if (schemaQuery.isLoading) {
    return (
      <div className="space-y-6">
        <div className="flex items-center gap-4">
          <Link href={`/admin/content-types/${singular}`}>
            <Button variant="outline" size="sm">
              &larr; Back
            </Button>
          </Link>
          <Skeleton className="h-8 w-32" />
        </div>
        <Skeleton className="h-64 w-full" />
      </div>
    );
  }

  if (!schema) {
    return (
      <div className="space-y-6">
        <div className="flex items-center gap-4">
          <Link href="/admin/content-types">
            <Button variant="outline" size="sm">
              &larr; Back
            </Button>
          </Link>
          <h1 className="text-2xl font-bold">Not Found</h1>
        </div>
        <Card>
          <CardContent className="py-8 text-center text-muted-foreground">
            Content type &quot;{singular}&quot; not found.
          </CardContent>
        </Card>
      </div>
    );
  }

  return (
    <div className="space-y-6">
      <div className="flex items-center gap-4">
        <Link href={`/admin/content-types/${singular}`}>
          <Button variant="outline" size="sm">
            &larr; Back
          </Button>
        </Link>
        <h1 className="text-2xl font-bold">New {schema.name}</h1>
      </div>

      <form onSubmit={handleSubmit}>
        <div className="grid gap-6 lg:grid-cols-3">
          <div className="lg:col-span-2 space-y-6">
            <Card>
              <CardContent className="pt-6 space-y-4">
                {formFields.map((field) => (
                  <FieldRenderer
                    key={field.name}
                    field={field}
                    value={formData[field.name]}
                    onChange={(val) => handleChange(field.name, val)}
                    error={errors[field.name]}
                  />
                ))}
              </CardContent>
            </Card>
          </div>

          <div className="space-y-6">
            {schema.draft_publish && (
              <Card>
                <CardContent className="pt-6 space-y-4">
                  <div className="space-y-2">
                    <Label>Status</Label>
                    <Select
                      value={(formData["status"] as string) || "draft"}
                      onValueChange={(val) => {
                        if (val) handleChange("status", val);
                      }}
                    >
                      <SelectTrigger className="w-full">
                        <SelectValue placeholder="Select status" />
                      </SelectTrigger>
                      <SelectContent>
                        <SelectItem value="draft">Draft</SelectItem>
                        <SelectItem value="published">Published</SelectItem>
                      </SelectContent>
                    </Select>
                  </div>
                </CardContent>
              </Card>
            )}

            <Card>
              <CardContent className="pt-6 space-y-2">
                <Button
                  type="submit"
                  className="w-full"
                  disabled={createMutation.isPending}
                >
                  {createMutation.isPending ? "Creating..." : "Create"}
                </Button>
                <Link
                  href={`/admin/content-types/${singular}`}
                  className="block"
                >
                  <Button type="button" variant="outline" className="w-full">
                    Cancel
                  </Button>
                </Link>
              </CardContent>
            </Card>
          </div>
        </div>
      </form>
    </div>
  );
}
