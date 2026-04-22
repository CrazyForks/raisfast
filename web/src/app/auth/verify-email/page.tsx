"use client";

import { useState, useEffect, useRef } from "react";
import { useRouter, useSearchParams } from "next/navigation";
import Link from "next/link";
import { toast } from "sonner";

import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { api, ApiError } from "@/lib/api";

export default function VerifyEmailPage() {
  const router = useRouter();
  const searchParams = useSearchParams();
  const [status, setStatus] = useState<"loading" | "success" | "error">("loading");
  const [email, setEmail] = useState("");
  const [resending, setResending] = useState(false);
  const initialized = useRef(false);

  const token = searchParams.get("token") ?? "";

  useEffect(() => {
    if (initialized.current) return;
    initialized.current = true;

    if (!token) {
      setStatus("error");
      return;
    }

    api
      .post("/auth/verify-email", { token })
      .then(() => {
        setStatus("success");
        toast.success("Email verified successfully");
      })
      .catch((err) => {
        setStatus("error");
        if (err instanceof ApiError) {
          toast.error(err.message);
        } else {
          toast.error("Verification failed");
        }
      });
  }, [token]);

  async function handleResend() {
    if (!email) return;
    setResending(true);
    try {
      await api.post("/auth/resend-verification", { email });
      toast.success("Verification email sent");
    } catch (err) {
      if (err instanceof ApiError) {
        toast.error(err.message);
      } else {
        toast.error("Failed to resend");
      }
    } finally {
      setResending(false);
    }
  }

  if (status === "loading") {
    return (
      <Card>
        <CardHeader>
          <CardTitle className="text-center text-2xl">Verifying Email</CardTitle>
        </CardHeader>
        <CardContent className="text-center text-muted-foreground">
          Please wait…
        </CardContent>
      </Card>
    );
  }

  if (status === "success") {
    return (
      <Card>
        <CardHeader>
          <CardTitle className="text-center text-2xl">Email Verified!</CardTitle>
        </CardHeader>
        <CardContent className="space-y-4 text-center">
          <p className="text-muted-foreground">
            Your email has been verified successfully.
          </p>
          <Link href="/auth/login">
            <Button className="w-full">Go to Login</Button>
          </Link>
        </CardContent>
      </Card>
    );
  }

  return (
    <Card>
      <CardHeader>
        <CardTitle className="text-center text-2xl">Verification Failed</CardTitle>
      </CardHeader>
      <CardContent className="space-y-4">
        <p className="text-center text-muted-foreground">
          The verification link is invalid or has expired. Enter your email to receive a new one.
        </p>
        <div className="flex gap-2">
          <input
            type="email"
            placeholder="you@example.com"
            value={email}
            onChange={(e) => setEmail(e.target.value)}
            className="flex h-10 w-full rounded-md border border-input bg-background px-3 py-2 text-sm"
          />
          <Button onClick={handleResend} disabled={resending || !email}>
            {resending ? "Sending…" : "Resend"}
          </Button>
        </div>
        <div className="text-center">
          <Link href="/auth/login" className="text-sm text-muted-foreground hover:underline">
            ← Back to login
          </Link>
        </div>
      </CardContent>
    </Card>
  );
}
