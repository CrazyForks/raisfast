"use client";

import { useState } from "react";
import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import { Plus, Trash2, Pencil, Save, X, Webhook } from "lucide-react";
import { useForm } from "react-hook-form";
import { z } from "zod";
import { zodResolver } from "@hookform/resolvers/zod";
import { toast } from "sonner";

import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
  DialogTrigger,
} from "@/components/ui/dialog";
import { Badge } from "@/components/ui/badge";
import { api, ApiError } from "@/lib/api";

interface WebhookSubscription {
  id: string;
  url: string;
  secret: string;
  events: string;
  enabled: boolean;
  description: string | null;
  created_at: string;
  updated_at: string;
}

interface PaginatedData<T> {
  items: T[];
  total: number;
  page: number;
  page_size: number;
}

const webhookSchema = z.object({
  url: z.string().url("Must be a valid URL"),
  events: z.string().min(1, "At least one event is required"),
  description: z.string().optional(),
});

type WebhookForm = z.infer<typeof webhookSchema>;

export default function WebhooksPage() {
  const queryClient = useQueryClient();
  const [dialogOpen, setDialogOpen] = useState(false);
  const [editWebhook, setEditWebhook] = useState<WebhookSubscription | null>(null);
  const [editUrl, setEditUrl] = useState("");
  const [editEvents, setEditEvents] = useState("");
  const [editDescription, setEditDescription] = useState("");
  const [editEnabled, setEditEnabled] = useState(true);
  const [page, setPage] = useState(1);
  const pageSize = 20;

  const webhooksQuery = useQuery({
    queryKey: ["webhooks", page],
    queryFn: () =>
      api.get<PaginatedData<WebhookSubscription>>(
        `/admin/webhooks?page=${page}&page_size=${pageSize}`,
      ),
  });

  const {
    register,
    handleSubmit,
    reset,
    formState: { errors },
  } = useForm<WebhookForm>({
    resolver: zodResolver(webhookSchema as never),
    defaultValues: { url: "", events: "post.created,post.updated", description: "" },
  });

  const createMutation = useMutation({
    mutationFn: (data: WebhookForm) =>
      api.post("/admin/webhooks", {
        url: data.url,
        events: data.events.split(",").map((e) => e.trim()).filter(Boolean),
        description: data.description || null,
      }),
    onSuccess: () => {
      toast.success("Webhook created");
      queryClient.invalidateQueries({ queryKey: ["webhooks"] });
      setDialogOpen(false);
      reset();
    },
    onError: (err) => {
      if (err instanceof ApiError) {
        toast.error(err.message);
      } else {
        toast.error("Failed to create webhook");
      }
    },
  });

  const updateMutation = useMutation({
    mutationFn: ({
      id,
      data,
    }: {
      id: string;
      data: {
        url?: string;
        events?: string[];
        description?: string;
        enabled?: boolean;
      };
    }) => api.put(`/admin/webhooks/${id}`, data),
    onSuccess: () => {
      toast.success("Webhook updated");
      queryClient.invalidateQueries({ queryKey: ["webhooks"] });
      setEditWebhook(null);
    },
    onError: (err) => {
      if (err instanceof ApiError) {
        toast.error(err.message);
      } else {
        toast.error("Failed to update webhook");
      }
    },
  });

  const deleteMutation = useMutation({
    mutationFn: (id: string) => api.delete(`/admin/webhooks/${id}`),
    onSuccess: () => {
      toast.success("Webhook deleted");
      queryClient.invalidateQueries({ queryKey: ["webhooks"] });
    },
    onError: (err) => {
      if (err instanceof ApiError) {
        toast.error(err.message);
      } else {
        toast.error("Failed to delete webhook");
      }
    },
  });

  function handleDelete(id: string) {
    if (confirm("Delete this webhook subscription?")) {
      deleteMutation.mutate(id);
    }
  }

  function startEdit(w: WebhookSubscription) {
    setEditWebhook(w);
    setEditUrl(w.url);
    setEditEvents(w.events);
    setEditDescription(w.description ?? "");
    setEditEnabled(w.enabled);
  }

  function saveEdit() {
    if (!editWebhook) return;
    const eventsArr = editEvents
      .split(",")
      .map((e) => e.trim())
      .filter(Boolean);
    updateMutation.mutate({
      id: editWebhook.id,
      data: {
        url: editUrl,
        events: eventsArr,
        description: editDescription || undefined,
        enabled: editEnabled,
      },
    });
  }

  function parseEvents(eventsStr: string): string[] {
    try {
      return JSON.parse(eventsStr);
    } catch {
      return [];
    }
  }

  const webhooks = webhooksQuery.data?.items ?? [];
  const totalPages = Math.ceil((webhooksQuery.data?.total ?? 0) / pageSize);

  return (
    <div className="space-y-6">
      <div className="flex items-center justify-between">
        <div className="flex items-center gap-2">
          <Webhook className="size-6" />
          <h1 className="text-2xl font-bold">Webhooks</h1>
        </div>
        <Dialog open={dialogOpen} onOpenChange={setDialogOpen}>
          <DialogTrigger render={<Button />}>
            <Plus className="size-4" />
            New Webhook
          </DialogTrigger>
          <DialogContent>
            <DialogHeader>
              <DialogTitle>New Webhook</DialogTitle>
              <DialogDescription>
                Create a new webhook subscription to receive event notifications.
              </DialogDescription>
            </DialogHeader>
            <form
              onSubmit={handleSubmit((data) => createMutation.mutate(data))}
              className="space-y-4"
            >
              <div className="space-y-2">
                <Label htmlFor="wh-url">Callback URL</Label>
                <Input
                  id="wh-url"
                  placeholder="https://example.com/webhook"
                  {...register("url")}
                />
                {errors.url && (
                  <p className="text-sm text-red-500">{errors.url.message}</p>
                )}
              </div>
              <div className="space-y-2">
                <Label htmlFor="wh-events">Events (comma-separated)</Label>
                <Input
                  id="wh-events"
                  placeholder="post.created, post.updated, comment.created"
                  {...register("events")}
                />
                {errors.events && (
                  <p className="text-sm text-red-500">{errors.events.message}</p>
                )}
                <p className="text-xs text-muted-foreground">
                  Use <code>*</code> to subscribe to all events
                </p>
              </div>
              <div className="space-y-2">
                <Label htmlFor="wh-desc">Description (optional)</Label>
                <Input
                  id="wh-desc"
                  placeholder="Notify external service..."
                  {...register("description")}
                />
              </div>
              <DialogFooter>
                <Button
                  type="button"
                  variant="outline"
                  onClick={() => setDialogOpen(false)}
                >
                  Cancel
                </Button>
                <Button type="submit" disabled={createMutation.isPending}>
                  {createMutation.isPending ? "Creating..." : "Create"}
                </Button>
              </DialogFooter>
            </form>
          </DialogContent>
        </Dialog>
      </div>

      <Card>
        <CardContent className="p-0">
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead>URL</TableHead>
                <TableHead>Events</TableHead>
                <TableHead>Status</TableHead>
                <TableHead>Description</TableHead>
                <TableHead>Created</TableHead>
                <TableHead className="text-right">Actions</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {webhooksQuery.isLoading ? (
                <TableRow>
                  <TableCell colSpan={6} className="text-center py-8">
                    Loading...
                  </TableCell>
                </TableRow>
              ) : webhooks.length === 0 ? (
                <TableRow>
                  <TableCell colSpan={6} className="text-center py-8">
                    No webhooks found.
                  </TableCell>
                </TableRow>
              ) : (
                webhooks.map((w) => (
                  <TableRow key={w.id}>
                    <TableCell>
                      {editWebhook?.id === w.id ? (
                        <Input
                          value={editUrl}
                          onChange={(e) => setEditUrl(e.target.value)}
                          className="h-8 w-64"
                        />
                      ) : (
                        <span className="font-mono text-xs max-w-64 truncate block">
                          {w.url}
                        </span>
                      )}
                    </TableCell>
                    <TableCell>
                      {editWebhook?.id === w.id ? (
                        <Input
                          value={editEvents}
                          onChange={(e) => setEditEvents(e.target.value)}
                          className="h-8 w-48"
                        />
                      ) : (
                        <div className="flex flex-wrap gap-1">
                          {parseEvents(w.events).map((ev) => (
                            <Badge key={ev} variant="secondary" className="text-xs">
                              {ev}
                            </Badge>
                          ))}
                        </div>
                      )}
                    </TableCell>
                    <TableCell>
                      {editWebhook?.id === w.id ? (
                        <select
                          value={editEnabled ? "true" : "false"}
                          onChange={(e) => setEditEnabled(e.target.value === "true")}
                          className="h-8 rounded-md border border-input bg-background px-2 text-sm"
                        >
                          <option value="true">Enabled</option>
                          <option value="false">Disabled</option>
                        </select>
                      ) : w.enabled ? (
                        <Badge variant="default">Enabled</Badge>
                      ) : (
                        <Badge variant="destructive">Disabled</Badge>
                      )}
                    </TableCell>
                    <TableCell>
                      {editWebhook?.id === w.id ? (
                        <Input
                          value={editDescription}
                          onChange={(e) => setEditDescription(e.target.value)}
                          className="h-8 w-40"
                          placeholder="—"
                        />
                      ) : (
                        <span className="text-sm text-muted-foreground">
                          {w.description || "—"}
                        </span>
                      )}
                    </TableCell>
                    <TableCell>
                      {new Date(w.created_at).toLocaleDateString()}
                    </TableCell>
                    <TableCell className="text-right">
                      {editWebhook?.id === w.id ? (
                        <div className="flex items-center justify-end gap-1">
                          <Button
                            variant="ghost"
                            size="icon-sm"
                            onClick={saveEdit}
                            disabled={updateMutation.isPending}
                          >
                            <Save className="size-4" />
                          </Button>
                          <Button
                            variant="ghost"
                            size="icon-sm"
                            onClick={() => setEditWebhook(null)}
                          >
                            <X className="size-4" />
                          </Button>
                        </div>
                      ) : (
                        <div className="flex items-center justify-end gap-1">
                          <Button
                            variant="ghost"
                            size="icon-sm"
                            onClick={() => startEdit(w)}
                          >
                            <Pencil className="size-4" />
                          </Button>
                          <Button
                            variant="ghost"
                            size="icon-sm"
                            onClick={() => handleDelete(w.id)}
                            disabled={deleteMutation.isPending}
                          >
                            <Trash2 className="size-4" />
                          </Button>
                        </div>
                      )}
                    </TableCell>
                  </TableRow>
                ))
              )}
            </TableBody>
          </Table>
        </CardContent>
      </Card>

      {totalPages > 1 && (
        <div className="flex items-center justify-center gap-2">
          <Button
            variant="outline"
            size="sm"
            disabled={page <= 1}
            onClick={() => setPage((p) => p - 1)}
          >
            Previous
          </Button>
          <span className="text-sm text-muted-foreground">
            Page {page} of {totalPages}
          </span>
          <Button
            variant="outline"
            size="sm"
            disabled={page >= totalPages}
            onClick={() => setPage((p) => p + 1)}
          >
            Next
          </Button>
        </div>
      )}
    </div>
  );
}
