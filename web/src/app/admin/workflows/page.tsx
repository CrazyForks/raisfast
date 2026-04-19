"use client";

import { useState } from "react";
import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import { GitBranch, Plus, Trash2, Eye, Play, Pencil } from "lucide-react";
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
import { api, ApiError } from "@/lib/api";

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
  const [dialogOpen, setDialogOpen] = useState(false);

  const workflowsQuery = useQuery({
    queryKey: ["workflows"],
    queryFn: () => api.get<WorkflowDefinition[]>("/admin/workflows"),
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
      return api.post("/admin/workflows", {
        id: data.id,
        name: data.name,
        description: data.description || null,
        steps,
      });
    },
    onSuccess: () => {
      toast.success("Workflow created");
      queryClient.invalidateQueries({ queryKey: ["workflows"] });
      setDialogOpen(false);
      reset();
    },
    onError: (err) => {
      if (err instanceof ApiError) toast.error(err.message);
      else toast.error("Failed to create workflow");
    },
  });

  const deleteMutation = useMutation({
    mutationFn: (id: string) => api.delete(`/admin/workflows/${id}`),
    onSuccess: () => {
      toast.success("Workflow deleted");
      queryClient.invalidateQueries({ queryKey: ["workflows"] });
    },
    onError: (err) => {
      if (err instanceof ApiError) toast.error(err.message);
      else toast.error("Failed to delete workflow");
    },
  });

  function handleDelete(id: string, name: string) {
    if (confirm(`Delete workflow "${name}"?`)) {
      deleteMutation.mutate(id);
    }
  }

  const workflows = workflowsQuery.data ?? [];

  return (
    <div className="space-y-6">
      <div className="flex items-center justify-between">
        <div className="flex items-center gap-2">
          <GitBranch className="size-6" />
          <h1 className="text-2xl font-bold">Workflows</h1>
        </div>
        <div className="flex items-center gap-2">
          <Link href="/admin/workflows/instances">
            <Button variant="outline" size="sm">
              <Eye className="size-4" />
              Instances
            </Button>
          </Link>
          <Link href="/admin/workflows/editor">
            <Button variant="outline" size="sm">
              <GitBranch className="size-4" />
              Visual Editor
            </Button>
          </Link>
          <Dialog open={dialogOpen} onOpenChange={setDialogOpen}>
            <DialogTrigger render={<Button />}>
              <Plus className="size-4" />
              New Workflow
            </DialogTrigger>
            <DialogContent className="max-w-lg">
              <DialogHeader>
                <DialogTitle>Create Workflow</DialogTitle>
                <DialogDescription>
                  Define a new workflow with steps. Steps are configured as JSON.
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
                  <Label htmlFor="wf-name">Name</Label>
                  <Input id="wf-name" placeholder="e.g. Editorial Review" {...register("name")} />
                  {errors.name && <p className="text-sm text-red-500">{errors.name.message}</p>}
                </div>
                <div className="space-y-2">
                  <Label htmlFor="wf-desc">Description (optional)</Label>
                  <Input id="wf-desc" placeholder="Brief description..." {...register("description")} />
                </div>
                <div className="space-y-2">
                  <Label htmlFor="wf-steps">Steps (JSON)</Label>
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
                    Each step: {`{ id, name, type: "task"|"await"|"branch"|"parallel"|"delay", config, next }`}
                  </p>
                </div>
                <DialogFooter>
                  <Button type="button" variant="outline" onClick={() => setDialogOpen(false)}>
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
      </div>

      <Card>
        <CardContent className="p-0">
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead>Name</TableHead>
                <TableHead>ID</TableHead>
                <TableHead>Steps</TableHead>
                <TableHead>Status</TableHead>
                <TableHead>Created</TableHead>
                <TableHead className="text-right">Actions</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {workflowsQuery.isLoading ? (
                <TableRow>
                  <TableCell colSpan={6} className="text-center py-8">Loading...</TableCell>
                </TableRow>
              ) : workflows.length === 0 ? (
                <TableRow>
                  <TableCell colSpan={6} className="text-center py-8">
                    No workflows found. Create one to get started.
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
                      <Badge variant="secondary">{parseStepCount(wf.steps)} steps</Badge>
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
                      <div className="flex items-center justify-end gap-1">
                        <Link href={`/admin/workflows/editor?id=${wf.id}`}>
                          <Button variant="ghost" size="icon-sm" title="Open in visual editor">
                            <Pencil className="size-4" />
                          </Button>
                        </Link>
                        <Button
                          variant="ghost"
                          size="icon-sm"
                          onClick={() => {
                            const ctx = prompt("Enter context JSON:", "{}");
                            if (ctx) {
                              try {
                                const context = JSON.parse(ctx);
                                api
                                  .post(`/admin/workflows/${wf.id}/start`, { context })
                                  .then(() => {
                                    toast.success("Workflow started");
                                    queryClient.invalidateQueries({ queryKey: ["workflow-instances"] });
                                  })
                                  .catch((e: Error) => toast.error(e.message));
                              } catch {
                                toast.error("Invalid JSON");
                              }
                            }
                          }}
                          title="Start workflow"
                        >
                          <Play className="size-4" />
                        </Button>
                        <Button
                          variant="ghost"
                          size="icon-sm"
                          onClick={() => handleDelete(wf.id, wf.name)}
                          disabled={deleteMutation.isPending}
                        >
                          <Trash2 className="size-4" />
                        </Button>
                      </div>
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
