
import { useState, useEffect } from "react";
import Link from "@/lib/link";
import { useRouter, useParams } from "@/lib/navigation";
import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import { ArrowLeft } from "lucide-react";
import { toast } from "sonner";

import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Textarea } from "@/components/ui/textarea";
import { Skeleton } from "@/components/ui/skeleton";
import { page as pageApi } from "@/lib/page";
import { PageStatus } from "@raisfast/sdk";
import { useT } from "@/lib/i18n";
import { BlockEditor } from "@/components/admin/block-editor";

const TEMPLATES = ["default", "full", "landing", "contact"];

export default function EditPagePage() {
  const { id = "" } = useParams();
  const { t } = useT();
  const router = useRouter();
  const queryClient = useQueryClient();

  const pageQuery = useQuery({
    queryKey: ["admin-page", id],
    queryFn: () => pageApi.adminGet(id),
  });

  const [title, setTitle] = useState("");
  const [slug, setSlug] = useState("");
  const [status, setStatus] = useState<PageStatus>(PageStatus.draft);
  const [template, setTemplate] = useState("default");
  const [metaTitle, setMetaTitle] = useState("");
  const [metaDescription, setMetaDescription] = useState("");
  const [ogImage, setOgImage] = useState("");
  const [coverImage, setCoverImage] = useState("");
  const [content, setContent] = useState("");
  const [blocks, setBlocks] = useState<object[]>([]);
  const [mode, setMode] = useState<"blocks" | "markdown">("blocks");
  const [loaded, setLoaded] = useState(false);

  useEffect(() => {
    if (pageQuery.data && !loaded) {
      const d = pageQuery.data;
      setTitle(d.title);
      setSlug(d.slug);
      setStatus(d.status as PageStatus);
      setTemplate(d.template);
      setMetaTitle(d.meta_title ?? "");
      setMetaDescription(d.meta_description ?? "");
      setOgImage(d.og_image ?? "");
      setCoverImage(d.cover_image ?? "");
      if (d.content) {
        setContent(d.content);
        setMode("markdown");
      }
      if (d.blocks) {
        try {
          setBlocks(JSON.parse(d.blocks));
          setMode("blocks");
        } catch { /* ignore */ }
      }
      setLoaded(true);
    }
  }, [pageQuery.data, loaded]);

  const updateMutation = useMutation({
    mutationFn: (data: Record<string, unknown>) => pageApi.update(id, data),
    onSuccess: () => {
      toast.success(t("pages.pageUpdated"));
      queryClient.invalidateQueries({ queryKey: ["admin-page", id] });
    },
    onError: () => toast.error(t("pages.failedToUpdate")),
  });

  function handleSubmit(publish: boolean) {
    if (!title.trim()) {
      toast.error(t("common.titleRequired"));
      return;
    }
    updateMutation.mutate({
      title,
      slug,
      status: publish ? PageStatus.published : status,
      template,
      meta_title: metaTitle || null,
      meta_description: metaDescription || null,
      og_image: ogImage || null,
      cover_image: coverImage || null,
      content: mode === "markdown" ? content : null,
      blocks: mode === "blocks" ? JSON.stringify(blocks) : null,
    });
  }

  if (pageQuery.isLoading) {
    return <div className="space-y-4"><Skeleton className="h-8 w-48" /><Skeleton className="h-64" /></div>;
  }

  if (!pageQuery.data) {
    return <p className="text-muted-foreground">{t("common.notFound")}</p>;
  }

  return (
    <div className="space-y-6 max-w-4xl">
      <div className="flex items-center gap-4">
        <Link href="/pages"><Button variant="outline" size="sm"><ArrowLeft className="size-4" /></Button></Link>
        <h1 className="text-2xl font-bold">{t("pages.editPage")}</h1>
        <span className="text-sm text-muted-foreground capitalize">({status})</span>
      </div>

      <Card>
        <CardHeader><CardTitle>{t("pages.basicInfo")}</CardTitle></CardHeader>
        <CardContent className="space-y-4">
          <div>
            <Label>{t("pages.titleLabel")}</Label>
            <Input value={title} onChange={(e) => setTitle(e.target.value)} />
          </div>
          <div>
            <Label>{t("pages.slug")}</Label>
            <Input value={slug} onChange={(e) => setSlug(e.target.value)} />
          </div>
          <div className="grid grid-cols-2 gap-4">
            <div>
              <Label>{t("common.status")}</Label>
              <select className="flex h-9 w-full rounded-md border border-input bg-background px-3 py-1 text-sm" value={status} onChange={(e) => setStatus(e.target.value as PageStatus)}>
                <option value={PageStatus.draft}>Draft</option>
                <option value={PageStatus.published}>Published</option>
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
            <Input value={coverImage} onChange={(e) => setCoverImage(e.target.value)} />
          </div>
        </CardContent>
      </Card>

      <Card>
        <CardHeader><CardTitle>SEO</CardTitle></CardHeader>
        <CardContent className="space-y-4">
          <div><Label>{t("pages.metaTitle")}</Label><Input value={metaTitle} onChange={(e) => setMetaTitle(e.target.value)} /></div>
          <div><Label>{t("pages.metaDescription")}</Label><Textarea value={metaDescription} onChange={(e) => setMetaDescription(e.target.value)} rows={2} /></div>
          <div><Label>{t("pages.ogImage")}</Label><Input value={ogImage} onChange={(e) => setOgImage(e.target.value)} /></div>
        </CardContent>
      </Card>

      <Card>
        <CardHeader>
          <div className="flex items-center justify-between">
            <CardTitle>{t("pages.content")}</CardTitle>
            <div className="flex gap-1 bg-muted rounded-md p-0.5">
              <button type="button" onClick={() => setMode("blocks")} className={`px-3 py-1 text-xs rounded ${mode === "blocks" ? "bg-background shadow-sm" : "text-muted-foreground"}`}>{t("pages.blockMode")}</button>
              <button type="button" onClick={() => setMode("markdown")} className={`px-3 py-1 text-xs rounded ${mode === "markdown" ? "bg-background shadow-sm" : "text-muted-foreground"}`}>Markdown</button>
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
        <Button variant="outline" onClick={() => router.push("/pages")}>{t("common.cancel")}</Button>
        <Button onClick={() => handleSubmit(false)} disabled={updateMutation.isPending}>
          {updateMutation.isPending ? t("common.saving") : t("pages.saveDraft")}
        </Button>
        <Button onClick={() => handleSubmit(true)} disabled={updateMutation.isPending}>
          {t("pages.publish")}
        </Button>
      </div>
    </div>
  );
}
