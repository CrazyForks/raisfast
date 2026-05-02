"use client";

import { useState } from "react";
import { useRouter } from "next/navigation";
import { useForm } from "react-hook-form";
import { z } from "zod";
import { zodResolver } from "@hookform/resolvers/zod";
import { toast } from "sonner";

import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle, CardDescription } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { client } from "@/lib/raisfast";
import { SDKError } from "@raisfast/sdk";
import { useAuthStore } from "@/stores/auth";

const passwordSchema = z
  .object({
    old_password: z.string().min(1, "Current password is required"),
    new_password: z.string().min(8, "Password must be at least 8 characters"),
    confirm_password: z.string().min(1, "Please confirm your password"),
  })
  .refine((data) => data.new_password === data.confirm_password, {
    message: "Passwords do not match",
    path: ["confirm_password"],
  });

type PasswordForm = z.infer<typeof passwordSchema>;

export default function SecurityPage() {
  const router = useRouter();
  const { logout } = useAuthStore();
  const [loading, setLoading] = useState(false);

  const {
    register,
    handleSubmit,
    reset,
    formState: { errors },
  } = useForm<PasswordForm>({
    resolver: zodResolver(passwordSchema as never),
  });

  async function onSubmit(values: PasswordForm) {
    setLoading(true);
    try {
      await client.users.changePassword({
        old_password: values.old_password,
        new_password: values.new_password,
      });
      toast.success("Password changed. Please log in again.");
      logout();
      router.push("/auth/login");
    } catch (err) {
      if (err instanceof SDKError) {
        toast.error(err.message);
      } else {
        toast.error("Failed to change password");
      }
    } finally {
      setLoading(false);
    }
  }

  return (
    <div className="space-y-6">
      <div>
        <h1 className="text-2xl font-bold">Security</h1>
        <p className="text-muted-foreground">Manage your password and security settings.</p>
      </div>

      <Card>
        <CardHeader>
          <CardTitle>Change Password</CardTitle>
          <CardDescription>
            After changing your password, you will be logged out and need to sign in again.
          </CardDescription>
        </CardHeader>
        <CardContent>
          <form onSubmit={handleSubmit(onSubmit)} className="space-y-4">
            <div className="space-y-2">
              <Label htmlFor="old_password">Current Password</Label>
              <Input id="old_password" type="password" {...register("old_password")} />
              {errors.old_password && <p className="text-sm text-red-500">{errors.old_password.message}</p>}
            </div>

            <div className="space-y-2">
              <Label htmlFor="new_password">New Password</Label>
              <Input id="new_password" type="password" {...register("new_password")} />
              {errors.new_password && <p className="text-sm text-red-500">{errors.new_password.message}</p>}
            </div>

            <div className="space-y-2">
              <Label htmlFor="confirm_password">Confirm New Password</Label>
              <Input id="confirm_password" type="password" {...register("confirm_password")} />
              {errors.confirm_password && <p className="text-sm text-red-500">{errors.confirm_password.message}</p>}
            </div>

            <Button type="submit" variant="destructive" disabled={loading}>
              {loading ? "Changing..." : "Change Password"}
            </Button>
          </form>
        </CardContent>
      </Card>
    </div>
  );
}
