"use client";

import { useState } from "react";
import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import { GitBranch, Plus, Trash2, Eye, Play, Pencil, MoreVertical } from "lucide-react";
import { useForm } from "react-hook-form";
import { z } from "zod";
import { zodResolver } from "@hookform/resolvers/zod";
import { toast } from "sonner";
import Link from "next/link";

import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Textarea } from "@/components/ui/textarea";
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
import { DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuTrigger } from "@/components/ui/dropdown-menu";
import { client } from "@/lib/raisfast";
import { SDKError } from "@raisfast/sdk";
import { useT } from "@/lib/i18n";

interface WorkflowDefinition {
  id: string;
  name: string;
  description: string | null;
  steps: string;
  initial_step: string;
  version: number;
  enabled: boolean;
  created_at: string;
  updated_at: string;
}

const createSchema = z.object({
  id: z.string().min(1, "ID is required"),
  name: z.string().min(1, "Name is required"),
  description: z.string().optional(),
  steps_json: z.string().min(1, "Steps JSON is required"),
});

type CreateForm = z.infer<typeof createSchema>;

const DEFAULT_STEPS = JSON.stringify(
  [
    { id: "s1", name: "Step 1", type: "task", config: {}, next: "" },
  ],
  null,
  2,
);

function parseStepCount(stepsJson: string): number {
  try {
    return JSON.parse(stepsJson).length;
  } catch {
    return 0;
  }
}

export default function WorkflowsPage() {
  const queryClient = useQueryClient();
  const { t } = useT();
  const [dialogOpen, setDialogOpen] = useState(false);

  const workflowsQuery = useQuery({
    queryKey: ["workflows"],
    queryFn: () => client.send<WorkflowDefinition[]>("/admin/workflows"),
  });

  const {
    register,
    handleSubmit,
    reset,
    formState: { errors },
  } = useForm<CreateForm>({
    resolver: zodResolver(createSchema as never),
    defaultValues: { id: "", name: "", description: "", steps_json: DEFAULT_STEPS },
  });

  const createMutation = useMutation({
    mutationFn: (data: CreateForm) => {
      const steps = JSON.parse(data.steps_json);
      return client.admin.workflows.create({
        name: data.name,
        steps,
      });
    },
    onSuccess: () => {
      toast.success(t("workflows.workflowCreated"));
      queryClient.invalidateQueries({ queryKey: ["workflows"] });
      setDialogOpen(false);
      reset();
    },
    onError: (err) => {
      if (err instanceof SDKError) toast.error(err.message);
      else toast.error(t("workflows.failedToCreate"));
    },
  });

  const deleteMutation = useMutation({
    mutationFn: (id: string) => client.admin.workflows.delete(id),
    onSuccess: () => {
      toast.success(t("workflows.workflowDeleted"));
      queryClient.invalidateQueries({ queryKey: ["workflows"] });
    },
    onError: (err) => {
      if (err instanceof SDKError) toast.error(err.message);
      else toast.error(t("workflows.failedToDelete"));
    },
  });

  function handleDelete(id: string, name: string) {
    if (confirm(t("workflows.confirmDelete", { name }))) {
      deleteMutation.mutate(id);
    }
  }

  const workflows = workflowsQuery.data ?? [];

  return (
    <div className="space-y-6">
      <div className="flex items-center justify-between">
        <div className="flex items-center gap-2">
          <GitBranch className="size-6" />
          <h1 className="text-2xl font-bold">{t("workflows.title")}</h1>
        </div>
        <div className="flex items-center gap-2">
          <Link href="/admin/workflows/instances">
            <Button variant="outline" size="sm">
              <Eye className="size-4" />
              {t("workflows.instances")}
            </Button>
          </Link>
          <Link href="/admin/workflows/editor">
            <Button variant="outline" size="sm">
              <GitBranch className="size-4" />
              {t("workflows.visualEditor")}
            </Button>
          </Link>
          <Dialog open={dialogOpen} onOpenChange={setDialogOpen}>
            <DialogTrigger render={<Button />}>
              <Plus className="size-4" />
              {t("workflows.newWorkflow")}
            </DialogTrigger>
            <DialogContent className="max-w-lg">
              <DialogHeader>
                <DialogTitle>{t("workflows.createWorkflow")}</DialogTitle>
                <DialogDescription>
                  {t("workflows.createWorkflowDesc")}
                </DialogDescription>
              </DialogHeader>
              <form
                onSubmit={handleSubmit((data) => createMutation.mutate(data))}
                className="space-y-4"
              >
                <div className="space-y-2">
                  <Label htmlFor="wf-id">ID</Label>
                  <Input id="wf-id" placeholder="e.g. editorial-review" {...register("id")} />
                  {errors.id && <p className="text-sm text-red-500">{errors.id.message}</p>}
                </div>
                <div className="space-y-2">
                  <Label htmlFor="wf-name">{t("common.name")}</Label>
                  <Input id="wf-name" placeholder="e.g. Editorial Review" {...register("name")} />
                  {errors.name && <p className="text-sm text-red-500">{errors.name.message}</p>}
                </div>
                <div className="space-y-2">
                  <Label htmlFor="wf-desc">Description (optional)</Label>
                  <Input id="wf-desc" placeholder="Brief description..." {...register("description")} />
                </div>
                <div className="space-y-2">
                  <Label htmlFor="wf-steps">{t("workflows.stepsJson")}</Label>
                  <Textarea
                    id="wf-steps"
                    rows={8}
                    className="font-mono text-xs"
                    {...register("steps_json")}
                  />
                  {errors.steps_json && (
                    <p className="text-sm text-red-500">{errors.steps_json.message}</p>
                  )}
                  <p className="text-xs text-muted-foreground">
                    {t("workflows.stepFormat")}
                  </p>
                </div>
                <DialogFooter>
                  <Button type="button" variant="outline" onClick={() => setDialogOpen(false)}>
                    {t("common.cancel")}
                  </Button>
                  <Button type="submit" disabled={createMutation.isPending}>
                    {createMutation.isPending ? t("common.creating") : t("common.create")}
                  </Button>
                </DialogFooter>
              </form>
            </DialogContent>
          </Dialog>
        </div>
      </div>

      <Card>
        <CardContent className="p-0">
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead>{t("common.name")}</TableHead>
                <TableHead>ID</TableHead>
                <TableHead>Steps</TableHead>
                <TableHead>{t("common.status")}</TableHead>
                <TableHead>Created</TableHead>
                <TableHead className="text-right">{t("common.actions")}</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {workflowsQuery.isLoading ? (
                <TableRow>
                  <TableCell colSpan={6} className="text-center py-8">{t("common.loading")}</TableCell>
                </TableRow>
              ) : workflows.length === 0 ? (
                <TableRow>
                  <TableCell colSpan={6} className="text-center py-8">
                    {t("workflows.noWorkflows")}
                  </TableCell>
                </TableRow>
              ) : (
                workflows.map((wf) => (
                  <TableRow key={wf.id}>
                    <TableCell className="font-medium">{wf.name}</TableCell>
                    <TableCell>
                      <code className="text-xs bg-muted px-1.5 py-0.5 rounded">{wf.id}</code>
                    </TableCell>
                    <TableCell>
                      <Badge variant="secondary">{t("workflows.steps", { count: parseStepCount(wf.steps) })}</Badge>
                    </TableCell>
                    <TableCell>
                      {wf.enabled ? (
                        <Badge variant="default">Enabled</Badge>
                      ) : (
                        <Badge variant="destructive">Disabled</Badge>
                      )}
                    </TableCell>
                    <TableCell>{new Date(wf.created_at).toLocaleDateString()}</TableCell>
                    <TableCell className="text-right">
                      <DropdownMenu>
                        <DropdownMenuTrigger className="inline-flex items-center justify-center rounded-md p-1 hover:bg-muted transition-colors">
                          <MoreVertical className="size-4" />
                        </DropdownMenuTrigger>
                        <DropdownMenuContent align="end">
                          <DropdownMenuItem onClick={() => window.location.href = `/admin/workflows/editor?id=${wf.id}`}>
                            <Pencil className="size-4 mr-2" />
                            Edit
                          </DropdownMenuItem>
                          <DropdownMenuItem
                            onClick={() => {
                              const ctx = prompt("Enter context JSON:", "{}");
                              if (ctx) {
                                try {
                                  const context = JSON.parse(ctx);
                                  client
                                    .send(`/admin/workflows/${wf.id}/start`, { method: "POST", body: { context } })
                                    .then(() => {
                                      toast.success(t("workflows.workflowStarted"));
                                      queryClient.invalidateQueries({ queryKey: ["workflow-instances"] });
                                    })
                                    .catch((e: Error) => toast.error(e.message));
                                } catch {
                                  toast.error(t("common.invalidJson"));
                                }
                              }
                            }}
                          >
                            <Play className="size-4 mr-2" />
                            Start workflow
                          </DropdownMenuItem>
                          <DropdownMenuItem
                            className="text-destructive"
                            onClick={() => handleDelete(wf.id, wf.name)}
                            disabled={deleteMutation.isPending}
                          >
                            <Trash2 className="size-4 mr-2" />
                            Delete
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
    </div>
  );
}
