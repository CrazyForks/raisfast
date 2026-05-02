"use client";

import { useState, useEffect, useRef } from "react";
import { useRouter, useSearchParams } from "next/navigation";
import Link from "next/link";
import { useForm } from "react-hook-form";
import { z } from "zod";
import { zodResolver } from "@hookform/resolvers/zod";
import { toast } from "sonner";

import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { client } from "@/lib/raisfast";
import { SDKError } from "@raisfast/sdk";

const resetSchema = z
  .object({
    password: z.string().min(8, "Password must be at least 8 characters").max(128),
    confirmPassword: z.string(),
  })
  .refine((data) => data.password === data.confirmPassword, {
    message: "Passwords do not match",
    path: ["confirmPassword"],
  });

type ResetForm = z.infer<typeof resetSchema>;

export default function ResetPasswordPage() {
  const router = useRouter();
  const searchParams = useSearchParams();
  const [loading, setLoading] = useState(false);
  const [invalidToken, setInvalidToken] = useState(false);
  const initialized = useRef(false);

  const token = searchParams.get("token") ?? "";

  useEffect(() => {
    if (initialized.current) return;
    initialized.current = true;
    if (!token) {
      setInvalidToken(true);
    }
  }, [token]);

  const {
    register,
    handleSubmit,
    formState: { errors },
  } = useForm<ResetForm>({ resolver: zodResolver(resetSchema as never) });

  async function onSubmit(values: ResetForm) {
    setLoading(true);
    try {
      await client.auth.confirmPasswordReset({
        token,
        new_password: values.password,
      });
      toast.success("Password reset successfully");
      router.push("/auth/login");
    } catch (err) {
      if (err instanceof SDKError) {
        toast.error(err.message);
        if (err.message.includes("token") || err.code >= 40000) {
          setInvalidToken(true);
        }
      } else {
        toast.error("An unexpected error occurred");
      }
    } finally {
      setLoading(false);
    }
  }

  if (invalidToken) {
    return (
      <Card>
        <CardHeader>
          <CardTitle className="text-center text-2xl">Invalid or Expired Link</CardTitle>
        </CardHeader>
        <CardContent className="space-y-4 text-center">
          <p className="text-muted-foreground">
            This password reset link is invalid or has expired.
          </p>
          <Link href="/auth/forgot-password" className="text-blue-600 hover:underline text-sm">
            Request a new reset link
          </Link>
        </CardContent>
      </Card>
    );
  }

  return (
    <Card>
      <CardHeader>
        <CardTitle className="text-center text-2xl">Reset Password</CardTitle>
      </CardHeader>
      <CardContent>
        <form onSubmit={handleSubmit(onSubmit)} className="space-y-4">
          <div className="space-y-2">
            <Label htmlFor="password">New Password</Label>
            <Input id="password" type="password" placeholder="••••••••" {...register("password")} />
            {errors.password && <p className="text-sm text-red-500">{errors.password.message}</p>}
          </div>

          <div className="space-y-2">
            <Label htmlFor="confirmPassword">Confirm Password</Label>
            <Input
              id="confirmPassword"
              type="password"
              placeholder="••••••••"
              {...register("confirmPassword")}
            />
            {errors.confirmPassword && (
              <p className="text-sm text-red-500">{errors.confirmPassword.message}</p>
            )}
          </div>

          <Button type="submit" className="w-full" disabled={loading}>
            {loading ? "Resetting…" : "Reset Password"}
          </Button>
        </form>

        <div className="mt-4 text-center">
          <Link href="/auth/login" className="text-sm text-muted-foreground hover:underline">
            ← Back to login
          </Link>
        </div>
      </CardContent>
    </Card>
  );
}
