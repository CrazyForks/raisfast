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
import { useAuthStore } from "@/stores/auth";
import { client } from "@/lib/raisfast";
import { SDKError } from "@raisfast/sdk";
import { OAuthButtons } from "@/components/auth/oauth-buttons";
import { SmsLoginForm } from "@/components/auth/sms-login-form";
import { useAuthConfig } from "@/hooks/use-auth-config";

const loginSchema = z.object({
  email: z.string().email("Invalid email address"),
  password: z.string().min(1, "Password is required"),
});

type LoginForm = z.infer<typeof loginSchema>;

export default function LoginPage() {
  const router = useRouter();
  const store = useAuthStore();
  const [loading, setLoading] = useState(false);
  const { config } = useAuthConfig();
  const [tab, setTab] = useState<"email" | "sms">("email");
  const [unverifiedEmail, setUnverifiedEmail] = useState<string | null>(null);
  const [resending, setResending] = useState(false);

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
  } = useForm<LoginForm>({ resolver: zodResolver(loginSchema as never) });

  async function onSubmit(values: LoginForm) {
    setLoading(true);
    try {
      const data = await client.auth.login(values.email, values.password);
      store.login(
        { id: data.user.id, email: data.user.email, username: data.user.nickname, role: data.user.role, avatar: data.user.avatar, bio: null },
        data.access_token,
        data.refresh_token,
      );
      toast.success("Logged in successfully");
      router.push("/");
    } catch (err) {
      if (err instanceof SDKError && err.message.includes("email_not_verified")) {
        setUnverifiedEmail(values.email);
      } else if (err instanceof SDKError) {
        toast.error(err.message);
      } else {
        toast.error("An unexpected error occurred");
      }
    } finally {
      setLoading(false);
    }
  }

  async function handleResend() {
    if (!unverifiedEmail) return;
    setResending(true);
    try {
      await client.send("/auth/resend-verification", { method: "POST", body: { email: unverifiedEmail } });
      toast.success("Verification email sent");
      setUnverifiedEmail(null);
    } catch (err) {
      if (err instanceof SDKError) {
        toast.error(err.message);
      } else {
        toast.error("Failed to resend");
      }
    } finally {
      setResending(false);
    }
  }

  return (
    <Card>
      <CardHeader>
        <CardTitle className="text-center text-2xl">Login</CardTitle>
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
          <>
            {unverifiedEmail && (
              <div className="rounded-md border border-yellow-300 bg-yellow-50 p-3 text-sm text-yellow-800 dark:border-yellow-700 dark:bg-yellow-950 dark:text-yellow-200">
                <p>Your email has not been verified. Please check your inbox or resend the verification email.</p>
                <Button
                  type="button"
                  variant="outline"
                  size="sm"
                  className="mt-2"
                  disabled={resending}
                  onClick={handleResend}
                >
                  {resending ? "Sending…" : "Resend verification email"}
                </Button>
              </div>
            )}
          <form onSubmit={handleSubmit(onSubmit)} className="space-y-4">
            <div className="space-y-2">
              <Label htmlFor="email">Email</Label>
              <Input id="email" type="email" placeholder="you@example.com" {...register("email")} />
              {errors.email && <p className="text-sm text-red-500">{errors.email.message}</p>}
            </div>

            <div className="space-y-2">
              <Label htmlFor="password">Password</Label>
              <Input id="password" type="password" placeholder="••••••••" {...register("password")} />
              {errors.password && <p className="text-sm text-red-500">{errors.password.message}</p>}
            </div>

            <div className="flex items-center justify-end">
              <Link href="/auth/forgot-password" className="text-sm text-muted-foreground hover:underline">
                Forgot password?
              </Link>
            </div>

            <Button type="submit" className="w-full" disabled={loading}>
              {loading ? "Logging in…" : "Login"}
            </Button>
          </form>
          </>
        )}

        {tab === "sms" && config.registration_sms_enabled && <SmsLoginForm />}

        <div className="mt-4 text-center text-sm">
          Don&apos;t have an account?{" "}
          <Link href="/auth/register" className="text-blue-600 hover:underline">
            Register
          </Link>
        </div>

        <div className="mt-2 text-center">
          <Link href="/" className="text-sm text-muted-foreground hover:underline">
            ← Back to home
          </Link>
        </div>
      </CardContent>
    </Card>
  );
}
