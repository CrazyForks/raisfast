
import { useState } from "react";
import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import { Plus, Trash2, Copy, Check, KeyRound, MoreVertical } from "lucide-react";
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
import { DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuTrigger } from "@/components/ui/dropdown-menu";
import { client } from "@/lib/raisfast";
import { SDKError } from "@raisfast/sdk";
import { useT } from "@/lib/i18n";

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
  const { t } = useT();
  const queryClient = useQueryClient();
  const [dialogOpen, setDialogOpen] = useState(false);
  const [revealedToken, setRevealedToken] = useState<CreateTokenResponse | null>(null);
  const [copied, setCopied] = useState(false);

  const tokensQuery = useQuery({
    queryKey: ["api-tokens"],
    queryFn: () => client.send<ApiToken[]>("/tokens"),
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
      client.send<CreateTokenResponse>("/tokens", {
        method: "POST",
        body: {
          name: data.name,
          scopes: data.scopes.split(",").map((s) => s.trim()).filter(Boolean),
          expires_at: data.expires_at || null,
        },
      }),
    onSuccess: (result) => {
      toast.success(t("tokens.tokenCreated"));
      queryClient.invalidateQueries({ queryKey: ["api-tokens"] });
      setDialogOpen(false);
      setRevealedToken(result);
      reset();
    },
    onError: (err) => {
      if (err instanceof SDKError) {
        toast.error(err.message);
      } else {
        toast.error(t("tokens.failedToCreate"));
      }
    },
  });

  const deleteMutation = useMutation({
    mutationFn: (id: string) => client.admin.tokens.delete(id),
    onSuccess: () => {
      toast.success(t("tokens.tokenRevoked"));
      queryClient.invalidateQueries({ queryKey: ["api-tokens"] });
    },
    onError: (err) => {
      if (err instanceof SDKError) {
        toast.error(err.message);
      } else {
        toast.error(t("tokens.failedToRevoke"));
      }
    },
  });

  function handleDelete(id: string, name: string) {
    if (confirm(t("tokens.confirmRevoke", { name }))) {
      deleteMutation.mutate(id);
    }
  }

  async function copyToken(token: string) {
    await navigator.clipboard.writeText(token);
    setCopied(true);
    setTimeout(() => setCopied(false), 2000);
    toast.success(t("tokens.tokenCopied"));
  }

  const tokens = tokensQuery.data ?? [];

  return (
    <div className="space-y-6">
      <div className="flex items-center justify-between">
        <div className="flex items-center gap-2">
          <KeyRound className="size-6" />
          <h1 className="text-2xl font-bold">{t("tokens.title")}</h1>
        </div>
        <Dialog open={dialogOpen} onOpenChange={setDialogOpen}>
          <DialogTrigger render={<Button />}>
            <Plus className="size-4" />
            {t("tokens.newToken")}
          </DialogTrigger>
          <DialogContent>
            <DialogHeader>
              <DialogTitle>{t("tokens.createToken")}</DialogTitle>
              <DialogDescription>
                {t("tokens.createTokenDesc")}
              </DialogDescription>
            </DialogHeader>
            <form
              onSubmit={handleSubmit((data) => createMutation.mutate(data))}
              className="space-y-4"
            >
              <div className="space-y-2">
                <Label htmlFor="tk-name">{t("common.name")}</Label>
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
                <Label htmlFor="tk-scopes">{t("tokens.scopesComma")}</Label>
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
                <Label htmlFor="tk-expires">{t("tokens.expiresOptional")}</Label>
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

      {revealedToken && (
        <Card className="border-yellow-500/50 bg-yellow-50 dark:bg-yellow-950/20">
          <CardContent className="p-4 space-y-3">
            <div className="flex items-center gap-2">
              <KeyRound className="size-4 text-yellow-600" />
              <span className="font-medium text-yellow-700 dark:text-yellow-400">
                {t("tokens.tokenCreatedCopy")}
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
                {copied ? t("tokens.copied") : t("tokens.copy")}
              </Button>
              <Button
                variant="ghost"
                size="sm"
                onClick={() => setRevealedToken(null)}
              >
                {t("tokens.dismiss")}
              </Button>
            </div>
            <code className="block rounded bg-background p-3 text-sm font-mono break-all select-all">
              {revealedToken.token}
            </code>
            <p className="text-xs text-muted-foreground">
              {t("tokens.notShownAgain")}
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
                <TableHead>{t("tokens.scopesCol")}</TableHead>
                <TableHead>{t("tokens.expiresCol")}</TableHead>
                <TableHead>{t("tokens.lastUsed")}</TableHead>
                <TableHead>Created</TableHead>
                <TableHead className="text-right">{t("common.actions")}</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {tokensQuery.isLoading ? (
                <TableRow>
                  <TableCell colSpan={6} className="text-center py-8">
                    {t("common.loading")}
                  </TableCell>
                </TableRow>
              ) : tokens.length === 0 ? (
                <TableRow>
                  <TableCell colSpan={6} className="text-center py-8">
                    {t("tokens.noTokens")}
                  </TableCell>
                </TableRow>
              ) : (
                tokens.map((tk) => (
                  <TableRow key={tk.id}>
                    <TableCell className="font-medium">{tk.name}</TableCell>
                    <TableCell>
                      <div className="flex flex-wrap gap-1">
                        {tk.scopes.map((s) => (
                          <ScopeBadge key={s} scope={s} />
                        ))}
                      </div>
                    </TableCell>
                    <TableCell>
                      {tk.expires_at
                        ? new Date(tk.expires_at).toLocaleDateString()
                        : <span className="text-muted-foreground">{t("tokens.never")}</span>}
                    </TableCell>
                    <TableCell>
                      {tk.last_used_at
                        ? new Date(tk.last_used_at).toLocaleString()
                        : <span className="text-muted-foreground">—</span>}
                    </TableCell>
                    <TableCell>
                      {new Date(tk.created_at).toLocaleDateString()}
                    </TableCell>
                    <TableCell className="text-right">
                      <DropdownMenu>
                        <DropdownMenuTrigger
                          className="inline-flex items-center justify-center rounded-md p-1 hover:bg-muted transition-colors"
                        >
                          <MoreVertical className="size-4" />
                        </DropdownMenuTrigger>
                        <DropdownMenuContent align="end">
                          <DropdownMenuItem
                            className="text-destructive"
                            onClick={() => handleDelete(tk.id, tk.name)}
                            disabled={deleteMutation.isPending}
                          >
                            <Trash2 className="size-4 mr-2" />
                            {t("common.delete")}
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
