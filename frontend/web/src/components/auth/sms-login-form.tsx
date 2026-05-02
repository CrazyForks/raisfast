"use client";

import { useState } from "react";
import { useRouter } from "next/navigation";
import { useForm } from "react-hook-form";
import { z } from "zod";
import { zodResolver } from "@hookform/resolvers/zod";
import { toast } from "sonner";

import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { useAuthStore } from "@/stores/auth";
import { client } from "@/lib/raisfast";
import { SDKError } from "@raisfast/sdk";
import { useSmsCountdown } from "@/hooks/use-auth-config";

const smsSchema = z.object({
  phone: z.string().min(5, "Phone number is required"),
  code: z.string().min(4, "Verification code is required"),
});

type SmsForm = z.infer<typeof smsSchema>;

export function SmsLoginForm() {
  const router = useRouter();
  const store = useAuthStore();
  const [loading, setLoading] = useState(false);
  const [sendingCode, setSendingCode] = useState(false);
  const { countdown, start } = useSmsCountdown();

  const {
    register,
    handleSubmit,
    getValues,
    formState: { errors },
  } = useForm<SmsForm>({ resolver: zodResolver(smsSchema as never) });

  async function handleSendCode() {
    const phone = getValues("phone");
    if (!phone || phone.length < 5) {
      toast.error("Please enter a valid phone number");
      return;
    }
    setSendingCode(true);
    try {
      await client.send("/auth/sms/send", { method: "POST", body: { phone, purpose: "login" } });
      start();
      toast.success("Verification code sent");
    } catch (err) {
      if (err instanceof SDKError) {
        toast.error(err.message);
      } else {
        toast.error("Failed to send code");
      }
    } finally {
      setSendingCode(false);
    }
  }

  async function onSubmit(values: SmsForm) {
    setLoading(true);
    try {
      const data = await client.send<{
        user: { id: string; email: string; username: string; role: string; avatar: string | null; bio: string | null };
        access_token: string;
        refresh_token: string;
      }>("/auth/sms/verify", {
        method: "POST",
        body: {
          phone: values.phone,
          code: values.code,
          purpose: "login",
        },
      });
      const u = data.user;
      store.login(
        { id: u.id, email: u.email, username: u.username, role: u.role, avatar: u.avatar, bio: u.bio },
        data.access_token,
        data.refresh_token,
      );
      toast.success("Logged in successfully");
      router.push("/");
    } catch (err) {
      if (err instanceof SDKError) {
        toast.error(err.message);
      } else {
        toast.error("An unexpected error occurred");
      }
    } finally {
      setLoading(false);
    }
  }

  return (
    <form onSubmit={handleSubmit(onSubmit)} className="space-y-4">
      <div className="space-y-2">
        <Label htmlFor="sms-phone">Phone Number</Label>
        <Input id="sms-phone" type="tel" placeholder="+86 138 xxxx xxxx" {...register("phone")} />
        {errors.phone && <p className="text-sm text-red-500">{errors.phone.message}</p>}
      </div>

      <div className="space-y-2">
        <Label htmlFor="sms-code">Verification Code</Label>
        <div className="flex gap-2">
          <Input id="sms-code" type="text" placeholder="123456" maxLength={6} {...register("code")} />
          <Button
            type="button"
            variant="outline"
            className="shrink-0"
            disabled={countdown > 0 || sendingCode}
            onClick={handleSendCode}
          >
            {countdown > 0 ? `${countdown}s` : sendingCode ? "Sending…" : "Send Code"}
          </Button>
        </div>
        {errors.code && <p className="text-sm text-red-500">{errors.code.message}</p>}
      </div>

      <Button type="submit" className="w-full" disabled={loading}>
        {loading ? "Logging in…" : "Login"}
      </Button>
    </form>
  );
}
