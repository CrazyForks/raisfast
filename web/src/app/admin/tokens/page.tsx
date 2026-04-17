"use client";

import { useState } from "react";
import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import { Plus, Trash2, Copy, Check, KeyRound } from "lucide-react";
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

interface ApiToken {
  id: string;
  name: string;
  scopes: string[];
  expires_at: string | null;
  last_used_at: string | null;
  created_at: string;
}

interface CreateTokenResponse {
  id: string;
  name: string;
  token: string;
  scopes: string[];
  expires_at: string | null;
  created_at: string;
}

const tokenSchema = z.object({
  name: z.string().min(1, "Name is required").max(100),
  scopes: z.string().min(1, "At least one scope is required"),
  expires_at: z.string().optional(),
});

type TokenForm = z.infer<typeof tokenSchema>;

const SCOPE_OPTIONS = [
  { value: "read", label: "Read", description: "Read access to content" },
  { value: "write", label: "Write", description: "Create and edit content" },
  { value: "admin", label: "Admin", description: "Full administrative access" },
];

function ScopeBadge({ scope }: { scope: string }) {
  const variant = scope === "admin" ? "destructive" : scope === "write" ? "default" : "secondary";
  return <Badge variant={variant}>{scope}</Badge>;
}

export default function ApiTokensPage() {
  const queryClient = useQueryClient();
  const [dialogOpen, setDialogOpen] = useState(false);
  const [revealedToken, setRevealedToken] = useState<CreateTokenResponse | null>(null);
  const [copied, setCopied] = useState(false);

  const tokensQuery = useQuery({
    queryKey: ["api-tokens"],
    queryFn: () => api.get<ApiToken[]>("/tokens"),
  });

  const {
    register,
    handleSubmit,
    reset,
    formState: { errors },
  } = useForm<TokenForm>({
    resolver: zodResolver(tokenSchema as never),
    defaultValues: { name: "", scopes: "read", expires_at: "" },
  });

  const createMutation = useMutation({
    mutationFn: (data: TokenForm) =>
      api.post<CreateTokenResponse>("/tokens", {
        name: data.name,
        scopes: data.scopes.split(",").map((s) => s.trim()).filter(Boolean),
        expires_at: data.expires_at || null,
      }),
    onSuccess: (result) => {
      toast.success("Token created");
      queryClient.invalidateQueries({ queryKey: ["api-tokens"] });
      setDialogOpen(false);
      setRevealedToken(result);
      reset();
    },
    onError: (err) => {
      if (err instanceof ApiError) {
        toast.error(err.message);
      } else {
        toast.error("Failed to create token");
      }
    },
  });

  const deleteMutation = useMutation({
    mutationFn: (id: string) => api.delete(`/tokens/${id}`),
    onSuccess: () => {
      toast.success("Token revoked");
      queryClient.invalidateQueries({ queryKey: ["api-tokens"] });
    },
    onError: (err) => {
      if (err instanceof ApiError) {
        toast.error(err.message);
      } else {
        toast.error("Failed to revoke token");
      }
    },
  });

  function handleDelete(id: string, name: string) {
    if (confirm(`Revoke token "${name}"? This cannot be undone.`)) {
      deleteMutation.mutate(id);
    }
  }

  async function copyToken(token: string) {
    await navigator.clipboard.writeText(token);
    setCopied(true);
    setTimeout(() => setCopied(false), 2000);
    toast.success("Token copied to clipboard");
  }

  const tokens = tokensQuery.data ?? [];

  return (
    <div className="space-y-6">
      <div className="flex items-center justify-between">
        <div className="flex items-center gap-2">
          <KeyRound className="size-6" />
          <h1 className="text-2xl font-bold">API Tokens</h1>
        </div>
        <Dialog open={dialogOpen} onOpenChange={setDialogOpen}>
          <DialogTrigger render={<Button />}>
            <Plus className="size-4" />
            New Token
          </DialogTrigger>
          <DialogContent>
            <DialogHeader>
              <DialogTitle>Create API Token</DialogTitle>
              <DialogDescription>
                Generate a new personal access token. The token value will only be shown once.
              </DialogDescription>
            </DialogHeader>
            <form
              onSubmit={handleSubmit((data) => createMutation.mutate(data))}
              className="space-y-4"
            >
              <div className="space-y-2">
                <Label htmlFor="tk-name">Name</Label>
                <Input
                  id="tk-name"
                  placeholder="e.g. CI/CD Pipeline"
                  {...register("name")}
                />
                {errors.name && (
                  <p className="text-sm text-red-500">{errors.name.message}</p>
                )}
              </div>
              <div className="space-y-2">
                <Label htmlFor="tk-scopes">Scopes (comma-separated)</Label>
                <div className="flex flex-wrap gap-2 mb-2">
                  {SCOPE_OPTIONS.map((opt) => (
                    <Badge
                      key={opt.value}
                      variant="outline"
                      className="cursor-pointer hover:bg-accent"
                    >
                      {opt.label}: {opt.description}
                    </Badge>
                  ))}
                </div>
                <Input
                  id="tk-scopes"
                  placeholder="read, write"
                  {...register("scopes")}
                />
                {errors.scopes && (
                  <p className="text-sm text-red-500">{errors.scopes.message}</p>
                )}
              </div>
              <div className="space-y-2">
                <Label htmlFor="tk-expires">Expires At (optional)</Label>
                <Input
                  id="tk-expires"
                  type="datetime-local"
                  {...register("expires_at")}
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

      {revealedToken && (
        <Card className="border-yellow-500/50 bg-yellow-50 dark:bg-yellow-950/20">
          <CardContent className="p-4 space-y-3">
            <div className="flex items-center gap-2">
              <KeyRound className="size-4 text-yellow-600" />
              <span className="font-medium text-yellow-700 dark:text-yellow-400">
                Token Created — copy it now!
              </span>
              <div className="flex-1" />
              <Button
                variant="outline"
                size="sm"
                onClick={() => copyToken(revealedToken.token)}
              >
                {copied ? (
                  <Check className="size-3.5" />
                ) : (
                  <Copy className="size-3.5" />
                )}
                {copied ? "Copied" : "Copy"}
              </Button>
              <Button
                variant="ghost"
                size="sm"
                onClick={() => setRevealedToken(null)}
              >
                Dismiss
              </Button>
            </div>
            <code className="block rounded bg-background p-3 text-sm font-mono break-all select-all">
              {revealedToken.token}
            </code>
            <p className="text-xs text-muted-foreground">
              This token will not be shown again. Store it securely.
            </p>
          </CardContent>
        </Card>
      )}

      <Card>
        <CardContent className="p-0">
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead>Name</TableHead>
                <TableHead>Scopes</TableHead>
                <TableHead>Expires</TableHead>
                <TableHead>Last Used</TableHead>
                <TableHead>Created</TableHead>
                <TableHead className="text-right">Actions</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {tokensQuery.isLoading ? (
                <TableRow>
                  <TableCell colSpan={6} className="text-center py-8">
                    Loading...
                  </TableCell>
                </TableRow>
              ) : tokens.length === 0 ? (
                <TableRow>
                  <TableCell colSpan={6} className="text-center py-8">
                    No API tokens found. Create one to get started.
                  </TableCell>
                </TableRow>
              ) : (
                tokens.map((t) => (
                  <TableRow key={t.id}>
                    <TableCell className="font-medium">{t.name}</TableCell>
                    <TableCell>
                      <div className="flex flex-wrap gap-1">
                        {t.scopes.map((s) => (
                          <ScopeBadge key={s} scope={s} />
                        ))}
                      </div>
                    </TableCell>
                    <TableCell>
                      {t.expires_at
                        ? new Date(t.expires_at).toLocaleDateString()
                        : <span className="text-muted-foreground">Never</span>}
                    </TableCell>
                    <TableCell>
                      {t.last_used_at
                        ? new Date(t.last_used_at).toLocaleString()
                        : <span className="text-muted-foreground">—</span>}
                    </TableCell>
                    <TableCell>
                      {new Date(t.created_at).toLocaleDateString()}
                    </TableCell>
                    <TableCell className="text-right">
                      <Button
                        variant="ghost"
                        size="icon-sm"
                        onClick={() => handleDelete(t.id, t.name)}
                        disabled={deleteMutation.isPending}
                      >
                        <Trash2 className="size-4" />
                      </Button>
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
