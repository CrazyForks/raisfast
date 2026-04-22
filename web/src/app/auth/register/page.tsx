"use client";

import { useState, useEffect } from "react";
import { useRouter } from "next/navigation";
import Link from "next/link";
import { useForm } from "react-hook-form";
import { z } from "zod";
import { zodResolver } from "@hookform/resolvers/zod";
import { toast } from "sonner";

import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { api, ApiError } from "@/lib/api";
import { OAuthButtons } from "@/components/auth/oauth-buttons";
import { SmsLoginForm } from "@/components/auth/sms-login-form";
import { useAuthConfig } from "@/hooks/use-auth-config";

const registerSchema = z
  .object({
    email: z.string().email("Invalid email address"),
    username: z.string().min(2, "Username must be at least 2 characters").max(50, "Username must be at most 50 characters"),
    password: z.string().min(8, "Password must be at least 8 characters").max(128, "Password must be at most 128 characters"),
    confirmPassword: z.string(),
  })
  .refine((data) => data.password === data.confirmPassword, {
    message: "Passwords do not match",
    path: ["confirmPassword"],
  });

type RegisterForm = z.infer<typeof registerSchema>;

export default function RegisterPage() {
  const router = useRouter();
  const [loading, setLoading] = useState(false);
  const { config } = useAuthConfig();
  const [tab, setTab] = useState<"email" | "sms">("email");

  useEffect(() => {
    if (config.registration_sms_enabled && !config.registration_email_enabled) {
      setTab("sms");
    }
  }, [config.registration_sms_enabled, config.registration_email_enabled]);

  const showTabs = config.registration_email_enabled && config.registration_sms_enabled;

  const {
    register,
    handleSubmit,
    formState: { errors },
  } = useForm<RegisterForm>({ resolver: zodResolver(registerSchema as never) });

  async function onSubmit(values: RegisterForm) {
    setLoading(true);
    try {
      await api.post("/auth/register", {
        email: values.email,
        username: values.username,
        password: values.password,
      });
      toast.success("Account created successfully");
      router.push("/auth/login");
    } catch (err) {
      if (err instanceof ApiError) {
        toast.error(err.message);
      } else {
        toast.error("An unexpected error occurred");
      }
    } finally {
      setLoading(false);
    }
  }

  return (
    <Card>
      <CardHeader>
        <CardTitle className="text-center text-2xl">Register</CardTitle>
      </CardHeader>
      <CardContent>
        <OAuthButtons />

        {showTabs && (
          <div className="flex mb-4 border-b">
            <button
              type="button"
              className={`flex-1 pb-2 text-sm font-medium border-b-2 transition-colors ${
                tab === "email"
                  ? "border-primary text-primary"
                  : "border-transparent text-muted-foreground hover:text-foreground"
              }`}
              onClick={() => setTab("email")}
            >
              Email
            </button>
            <button
              type="button"
              className={`flex-1 pb-2 text-sm font-medium border-b-2 transition-colors ${
                tab === "sms"
                  ? "border-primary text-primary"
                  : "border-transparent text-muted-foreground hover:text-foreground"
              }`}
              onClick={() => setTab("sms")}
            >
              Phone
            </button>
          </div>
        )}

        {tab === "email" && config.registration_email_enabled && (
          <form onSubmit={handleSubmit(onSubmit)} className="space-y-4">
            <div className="space-y-2">
              <Label htmlFor="email">Email</Label>
              <Input id="email" type="email" placeholder="you@example.com" {...register("email")} />
              {errors.email && <p className="text-sm text-red-500">{errors.email.message}</p>}
            </div>

            <div className="space-y-2">
              <Label htmlFor="username">Username</Label>
              <Input id="username" type="text" placeholder="johndoe" {...register("username")} />
              {errors.username && <p className="text-sm text-red-500">{errors.username.message}</p>}
            </div>

            <div className="space-y-2">
              <Label htmlFor="password">Password</Label>
              <Input id="password" type="password" placeholder="••••••••" {...register("password")} />
              {errors.password && <p className="text-sm text-red-500">{errors.password.message}</p>}
            </div>

            <div className="space-y-2">
              <Label htmlFor="confirmPassword">Confirm Password</Label>
              <Input id="confirmPassword" type="password" placeholder="••••••••" {...register("confirmPassword")} />
              {errors.confirmPassword && <p className="text-sm text-red-500">{errors.confirmPassword.message}</p>}
            </div>

            <Button type="submit" className="w-full" disabled={loading}>
              {loading ? "Creating account…" : "Register"}
            </Button>
          </form>
        )}

        {tab === "sms" && config.registration_sms_enabled && (
          <div className="space-y-4">
            <p className="text-sm text-muted-foreground text-center">
              Enter your phone number and verify to create an account.
            </p>
            <SmsLoginForm />
          </div>
        )}

        <div className="mt-4 text-center text-sm">
          Already have an account?{" "}
          <Link href="/auth/login" className="text-blue-600 hover:underline">
            Login
          </Link>
        </div>
      </CardContent>
    </Card>
  );
}
