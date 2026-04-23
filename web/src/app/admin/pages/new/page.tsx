"use client";

import { useState, useCallback } from "react";
import { useRouter } from "next/navigation";
import { useMutation } from "@tanstack/react-query";
import { toast } from "sonner";

import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Textarea } from "@/components/ui/textarea";
import { Separator } from "@/components/ui/separator";
import { page as pageApi } from "@/lib/page";
import { useT } from "@/lib/i18n";
import { BlockEditor } from "@/components/admin/block-editor";

const TEMPLATES = ["default", "full", "landing", "contact"];

export default function NewPagePage() {
  const { t } = useT();
  const router = useRouter();

  const [title, setTitle] = useState("");
  const [slug, setSlug] = useState("");
  const [status, setStatus] = useState("draft");
  const [template, setTemplate] = useState("default");
  const [metaTitle, setMetaTitle] = useState("");
  const [metaDescription, setMetaDescription] = useState("");
  const [ogImage, setOgImage] = useState("");
  const [coverImage, setCoverImage] = useState("");
  const [content, setContent] = useState("");
  const [blocks, setBlocks] = useState<object[]>([]);
  const [mode, setMode] = useState<"blocks" | "markdown">("blocks");

  const slugify = (s: string) =>
    s.toLowerCase().replace(/[^a-z0-9\u4e00-\u9fff]+/g, "-").replace(/^-|-$/g, "");

  const handleTitleChange = useCallback((v: string) => {
    setTitle(v);
    if (!slug || slug === slugify(title)) {
      setSlug(slugify(v));
    }
  }, [slug, title]);

  const createMutation = useMutation({
    mutationFn: (data: Record<string, unknown>) => pageApi.create(data),
    onSuccess: (res) => {
      toast.success(t("pages.pageCreated"));
      router.push(`/admin/pages/${res.id}/edit`);
    },
    onError: () => toast.error(t("pages.failedToCreate")),
  });

  function handleSubmit(publish: boolean) {
    if (!title.trim()) {
      toast.error(t("common.titleRequired"));
      return;
    }
    createMutation.mutate({
      title,
      slug: slug || slugify(title),
      status: publish ? "published" : status,
      template,
      meta_title: metaTitle || undefined,
      meta_description: metaDescription || undefined,
      og_image: ogImage || undefined,
      cover_image: coverImage || undefined,
      content: mode === "markdown" ? content : undefined,
      blocks: mode === "blocks" ? JSON.stringify(blocks) : undefined,
    });
  }

  return (
    <div className="space-y-6 max-w-4xl">
      <h1 className="text-2xl font-bold">{t("pages.newPage")}</h1>

      <Card>
        <CardHeader><CardTitle>{t("pages.basicInfo")}</CardTitle></CardHeader>
        <CardContent className="space-y-4">
          <div>
            <Label>{t("pages.titleLabel")}</Label>
            <Input value={title} onChange={(e) => handleTitleChange(e.target.value)} placeholder={t("pages.titlePlaceholder")} />
          </div>
          <div>
            <Label>{t("pages.slug")}</Label>
            <Input value={slug} onChange={(e) => setSlug(e.target.value)} placeholder="about-us" />
          </div>
          <div className="grid grid-cols-2 gap-4">
            <div>
              <Label>{t("common.status")}</Label>
              <select className="flex h-9 w-full rounded-md border border-input bg-background px-3 py-1 text-sm" value={status} onChange={(e) => setStatus(e.target.value)}>
                <option value="draft">Draft</option>
                <option value="published">Published</option>
              </select>
            </div>
            <div>
              <Label>{t("pages.template")}</Label>
              <select className="flex h-9 w-full rounded-md border border-input bg-background px-3 py-1 text-sm" value={template} onChange={(e) => setTemplate(e.target.value)}>
                {TEMPLATES.map((t) => <option key={t} value={t}>{t}</option>)}
              </select>
            </div>
          </div>
          <div>
            <Label>{t("pages.coverImage")}</Label>
            <Input value={coverImage} onChange={(e) => setCoverImage(e.target.value)} placeholder="/uploads/..." />
          </div>
        </CardContent>
      </Card>

      <Card>
        <CardHeader><CardTitle>SEO</CardTitle></CardHeader>
        <CardContent className="space-y-4">
          <div>
            <Label>{t("pages.metaTitle")}</Label>
            <Input value={metaTitle} onChange={(e) => setMetaTitle(e.target.value)} />
          </div>
          <div>
            <Label>{t("pages.metaDescription")}</Label>
            <Textarea value={metaDescription} onChange={(e) => setMetaDescription(e.target.value)} rows={2} />
          </div>
          <div>
            <Label>{t("pages.ogImage")}</Label>
            <Input value={ogImage} onChange={(e) => setOgImage(e.target.value)} placeholder="/uploads/..." />
          </div>
        </CardContent>
      </Card>

      <Card>
        <CardHeader>
          <div className="flex items-center justify-between">
            <CardTitle>{t("pages.content")}</CardTitle>
            <div className="flex gap-1 bg-muted rounded-md p-0.5">
              <button type="button" onClick={() => setMode("blocks")} className={`px-3 py-1 text-xs rounded ${mode === "blocks" ? "bg-background shadow-sm" : "text-muted-foreground"}`}>
                {t("pages.blockMode")}
              </button>
              <button type="button" onClick={() => setMode("markdown")} className={`px-3 py-1 text-xs rounded ${mode === "markdown" ? "bg-background shadow-sm" : "text-muted-foreground"}`}>
                Markdown
              </button>
            </div>
          </div>
        </CardHeader>
        <CardContent>
          {mode === "markdown" ? (
            <Textarea value={content} onChange={(e) => setContent(e.target.value)} rows={12} placeholder={t("pages.writeContent")} />
          ) : (
            <BlockEditor blocks={blocks} onChange={setBlocks} />
          )}
        </CardContent>
      </Card>

      <div className="flex gap-3">
        <Button variant="outline" onClick={() => router.push("/admin/pages")}>{t("common.cancel")}</Button>
        <Button onClick={() => handleSubmit(false)} disabled={createMutation.isPending}>
          {createMutation.isPending ? t("common.saving") : t("pages.saveDraft")}
        </Button>
        <Button onClick={() => handleSubmit(true)} disabled={createMutation.isPending}>
          {t("pages.publish")}
        </Button>
      </div>
    </div>
  );
}
