
import { useState } from "react";
import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import { Plus, Trash2, MoreVertical, Pencil } from "lucide-react";
import { toast } from "sonner";

import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogFooter,
} from "@/components/ui/dialog";
import { DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuTrigger } from "@/components/ui/dropdown-menu";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Textarea } from "@/components/ui/textarea";
import { Skeleton } from "@/components/ui/skeleton";
import { page as pageApi } from "@/lib/page";
import { useT } from "@/lib/i18n";

export default function ReusableBlocksPage() {
  const { t } = useT();
  const queryClient = useQueryClient();
  const [deleteId, setDeleteId] = useState<string | null>(null);
  const [editBlock, setEditBlock] = useState<{ id?: string; name: string; block_type: string; content: string; description: string } | null>(null);

  const listQuery = useQuery({
    queryKey: ["admin-reusable-blocks"],
    queryFn: () => pageApi.listReusable(),
  });

  const deleteMutation = useMutation({
    mutationFn: (id: string) => pageApi.deleteReusable(id),
    onSuccess: () => {
      toast.success(t("common.deleted"));
      queryClient.invalidateQueries({ queryKey: ["admin-reusable-blocks"] });
      setDeleteId(null);
    },
    onError: () => toast.error(t("common.deleteFailed")),
  });

  const saveMutation = useMutation({
    mutationFn: async (data: { name: string; block_type: string; content: string; description: string }) => {
      if (editBlock?.id) {
        return pageApi.updateReusable(editBlock.id, data);
      }
      return pageApi.createReusable(data);
    },
    onSuccess: () => {
      toast.success(editBlock?.id ? t("common.saved") : t("common.created"));
      queryClient.invalidateQueries({ queryKey: ["admin-reusable-blocks"] });
      setEditBlock(null);
    },
    onError: () => toast.error(t("common.saveFailed")),
  });

  const items = listQuery.data ?? [];

  return (
    <div className="space-y-6">
      <div className="flex items-center justify-between">
        <h1 className="text-2xl font-bold">{t("pages.reusableBlocks")}</h1>
        <Button onClick={() => setEditBlock({ name: "", block_type: "richtext", content: "{}", description: "" })}>
          <Plus className="size-4 mr-2" />
          {t("common.create")}
        </Button>
      </div>

      {listQuery.isLoading ? (
        <div className="space-y-3">
          <Skeleton className="h-10" />
          <Skeleton className="h-20" />
        </div>
      ) : (
        <Card>
          <CardContent className="p-0">
            <Table>
              <TableHeader>
                <TableRow>
                  <TableHead>{t("pages.rbName")}</TableHead>
                  <TableHead>{t("pages.rbType")}</TableHead>
                  <TableHead>{t("pages.rbDescription")}</TableHead>
                  <TableHead>{t("common.updatedAt")}</TableHead>
                  <TableHead className="w-12" />
                </TableRow>
              </TableHeader>
              <TableBody>
                {items.length === 0 ? (
                  <TableRow>
                    <TableCell colSpan={5} className="text-center text-muted-foreground py-8">
                      {t("common.noData")}
                    </TableCell>
                  </TableRow>
                ) : (
                  items.map((rb) => (
                    <TableRow key={rb.id}>
                      <TableCell className="font-medium">{rb.name}</TableCell>
                      <TableCell><span className="text-xs bg-muted px-2 py-0.5 rounded">{rb.block_type}</span></TableCell>
                      <TableCell className="text-sm text-muted-foreground max-w-xs truncate">{rb.description ?? "-"}</TableCell>
                      <TableCell className="text-sm text-muted-foreground">{new Date(rb.updated_at).toLocaleDateString()}</TableCell>
                      <TableCell>
                        <DropdownMenu>
                          <DropdownMenuTrigger render={<Button variant="ghost" size="icon-sm" />}>
                            <MoreVertical className="size-4" />
                          </DropdownMenuTrigger>
                          <DropdownMenuContent align="end">
                            <DropdownMenuItem onClick={() => setEditBlock({ id: rb.id, name: rb.name, block_type: rb.block_type, content: rb.content, description: rb.description ?? "" })}>
                              <Pencil className="size-3.5 mr-2" />{t("common.edit")}
                            </DropdownMenuItem>
                            <DropdownMenuItem className="text-destructive" onClick={() => setDeleteId(rb.id)}>
                              <Trash2 className="size-3.5 mr-2" />{t("common.delete")}
                            </DropdownMenuItem>
                          </DropdownMenuContent>
                        </DropdownMenu>
                      </TableCell>
                    </TableRow>
                  ))
                )}
              </TableBody>
            </Table>
          </CardContent>
        </Card>
      )}

      <Dialog open={!!deleteId} onOpenChange={() => setDeleteId(null)}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>{t("common.confirmDelete")}</DialogTitle>
          </DialogHeader>
          <DialogFooter>
            <Button variant="outline" onClick={() => setDeleteId(null)}>{t("common.cancel")}</Button>
            <Button variant="destructive" onClick={() => deleteId && deleteMutation.mutate(deleteId)}>{t("common.delete")}</Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      <Dialog open={!!editBlock} onOpenChange={() => setEditBlock(null)}>
        <DialogContent className="max-w-2xl">
          <DialogHeader>
            <DialogTitle>{editBlock?.id ? t("pages.editReusable") : t("pages.createReusable")}</DialogTitle>
          </DialogHeader>
          {editBlock && (
            <div className="space-y-4">
              <div className="grid grid-cols-2 gap-4">
                <div>
                  <Label>{t("pages.rbName")}</Label>
                  <Input value={editBlock.name} onChange={(e) => setEditBlock({ ...editBlock, name: e.target.value })} />
                </div>
                <div>
                  <Label>{t("pages.rbType")}</Label>
                  <select className="flex h-9 w-full rounded-md border border-input bg-background px-3 py-1 text-sm" value={editBlock.block_type} onChange={(e) => setEditBlock({ ...editBlock, block_type: e.target.value })}>
                    <option value="richtext">Text</option>
                    <option value="hero">Hero</option>
                    <option value="image">Image</option>
                    <option value="gallery">Gallery</option>
                    <option value="video">Video</option>
                    <option value="cta">CTA</option>
                    <option value="stats">Stats</option>
                    <option value="faq">FAQ</option>
                    <option value="testimonial">Testimonial</option>
                    <option value="timeline">Timeline</option>
                    <option value="team">Team</option>
                    <option value="pricing">Pricing</option>
                    <option value="contact_form">Form</option>
                    <option value="code">Code</option>
                    <option value="quote">Quote</option>
                    <option value="html">HTML</option>
                    <option value="custom">Custom</option>
                  </select>
                </div>
              </div>
              <div>
                <Label>{t("pages.rbDescription")}</Label>
                <Input value={editBlock.description} onChange={(e) => setEditBlock({ ...editBlock, description: e.target.value })} />
              </div>
              <div>
                <Label>{t("pages.rbContent")} (JSON)</Label>
                <Textarea rows={10} value={editBlock.content} onChange={(e) => setEditBlock({ ...editBlock, content: e.target.value })} className="font-mono text-xs" />
              </div>
            </div>
          )}
          <DialogFooter>
            <Button variant="outline" onClick={() => setEditBlock(null)}>{t("common.cancel")}</Button>
            <Button onClick={() => editBlock && saveMutation.mutate(editBlock)} disabled={saveMutation.isPending || !editBlock?.name}>
              {saveMutation.isPending ? t("common.saving") : t("common.save")}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </div>
  );
}
