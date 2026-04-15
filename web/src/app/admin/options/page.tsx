"use client";

import { useState } from "react";
import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import { Plus, Trash2, Pencil, Save, X } from "lucide-react";
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
import { api, ApiError } from "@/lib/api";

interface Option {
  key: string;
  value: string;
  public: boolean;
}

const optionSchema = z.object({
  key: z.string().min(1, "Key is required"),
  value: z.string().min(1, "Value is required"),
});

type OptionForm = z.infer<typeof optionSchema>;

export default function OptionsPage() {
  const queryClient = useQueryClient();
  const [dialogOpen, setDialogOpen] = useState(false);
  const [editingKey, setEditingKey] = useState<string | null>(null);
  const [editValue, setEditValue] = useState("");

  const optionsQuery = useQuery({
    queryKey: ["options"],
    queryFn: async () => {
      const data = await api.get<Record<string, unknown>>("/admin/options");
      return Object.entries(data).map(([key, value]) => ({
        key,
        value: typeof value === "string" ? value : JSON.stringify(value),
        public: false,
      }));
    },
  });

  const {
    register,
    handleSubmit,
    reset,
    formState: { errors },
  } = useForm<OptionForm>({
    resolver: zodResolver(optionSchema as never),
    defaultValues: { key: "", value: "" },
  });

  const createMutation = useMutation({
    mutationFn: (data: OptionForm) =>
      api.put(`/admin/options/${data.key}`, { value: data.value }),
    onSuccess: () => {
      toast.success("Option saved");
      queryClient.invalidateQueries({ queryKey: ["options"] });
      setDialogOpen(false);
      reset();
    },
    onError: (err) => {
      if (err instanceof ApiError) {
        toast.error(err.message);
      } else {
        toast.error("Failed to save option");
      }
    },
  });

  const updateMutation = useMutation({
    mutationFn: ({ key, value }: { key: string; value: string }) =>
      api.put(`/admin/options/${key}`, { value }),
    onSuccess: () => {
      toast.success("Option updated");
      queryClient.invalidateQueries({ queryKey: ["options"] });
      setEditingKey(null);
    },
    onError: (err) => {
      if (err instanceof ApiError) {
        toast.error(err.message);
      } else {
        toast.error("Failed to update option");
      }
    },
  });

  const deleteMutation = useMutation({
    mutationFn: (key: string) => api.delete(`/admin/options/${key}`),
    onSuccess: () => {
      toast.success("Option deleted");
      queryClient.invalidateQueries({ queryKey: ["options"] });
    },
    onError: (err) => {
      if (err instanceof ApiError) {
        toast.error(err.message);
      } else {
        toast.error("Failed to delete option");
      }
    },
  });

  function handleDelete(key: string) {
    if (confirm(`Delete option "${key}"?`)) {
      deleteMutation.mutate(key);
    }
  }

  function startEdit(key: string, value: string) {
    setEditingKey(key);
    setEditValue(value);
  }

  function cancelEdit() {
    setEditingKey(null);
    setEditValue("");
  }

  function saveEdit() {
    if (editingKey) {
      updateMutation.mutate({ key: editingKey, value: editValue });
    }
  }

  const options = optionsQuery.data ?? [];

  return (
    <div className="space-y-6">
      <div className="flex items-center justify-between">
        <h1 className="text-2xl font-bold">Options</h1>
        <Dialog open={dialogOpen} onOpenChange={setDialogOpen}>
          <DialogTrigger render={<Button />}>
            <Plus className="size-4" />
            New Option
          </DialogTrigger>
          <DialogContent>
            <DialogHeader>
              <DialogTitle>New Option</DialogTitle>
              <DialogDescription>
                Add a new site configuration option.
              </DialogDescription>
            </DialogHeader>
            <form
              onSubmit={handleSubmit((data) => createMutation.mutate(data))}
              className="space-y-4"
            >
              <div className="space-y-2">
                <Label htmlFor="opt-key">Key</Label>
                <Input
                  id="opt-key"
                  placeholder="site.title"
                  {...register("key")}
                />
                {errors.key && (
                  <p className="text-sm text-red-500">{errors.key.message}</p>
                )}
              </div>
              <div className="space-y-2">
                <Label htmlFor="opt-value">Value</Label>
                <Input
                  id="opt-value"
                  placeholder="My Blog"
                  {...register("value")}
                />
                {errors.value && (
                  <p className="text-sm text-red-500">
                    {errors.value.message}
                  </p>
                )}
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
                  {createMutation.isPending ? "Saving..." : "Save"}
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
                <TableHead className="w-[200px]">Key</TableHead>
                <TableHead>Value</TableHead>
                <TableHead className="w-[80px]">Public</TableHead>
                <TableHead className="w-[100px] text-right">Actions</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {optionsQuery.isLoading ? (
                <TableRow>
                  <TableCell colSpan={4} className="text-center py-8">
                    Loading...
                  </TableCell>
                </TableRow>
              ) : options.length === 0 ? (
                <TableRow>
                  <TableCell colSpan={4} className="text-center py-8">
                    No options found.
                  </TableCell>
                </TableRow>
              ) : (
                options.map((opt) => (
                  <TableRow key={opt.key}>
                    <TableCell className="font-mono text-sm font-medium">
                      {opt.key}
                    </TableCell>
                    <TableCell>
                      {editingKey === opt.key ? (
                        <div className="flex items-center gap-2">
                          <Input
                            value={editValue}
                            onChange={(e) => setEditValue(e.target.value)}
                            className="h-8"
                          />
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
                            onClick={cancelEdit}
                          >
                            <X className="size-4" />
                          </Button>
                        </div>
                      ) : (
                        <span className="text-sm">{opt.value}</span>
                      )}
                    </TableCell>
                    <TableCell>
                      {opt.public ? (
                        <span className="text-xs text-green-600 font-medium">
                          Yes
                        </span>
                      ) : (
                        <span className="text-xs text-muted-foreground">
                          No
                        </span>
                      )}
                    </TableCell>
                    <TableCell className="text-right">
                      <div className="flex items-center justify-end gap-1">
                        <Button
                          variant="ghost"
                          size="icon-sm"
                          onClick={() => startEdit(opt.key, opt.value)}
                          disabled={editingKey === opt.key}
                        >
                          <Pencil className="size-4" />
                        </Button>
                        <Button
                          variant="ghost"
                          size="icon-sm"
                          onClick={() => handleDelete(opt.key)}
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
