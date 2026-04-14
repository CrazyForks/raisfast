"use client";

import { useState } from "react";
import { useRouter } from "next/navigation";
import { useForm } from "react-hook-form";
import { z } from "zod";
import { zodResolver } from "@hookform/resolvers/zod";
import { toast } from "sonner";

import { Button } from "@/components/ui/button";
import {
  Card,
  CardContent,
  CardHeader,
  CardTitle,
  CardDescription,
} from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Textarea } from "@/components/ui/textarea";
import { Separator } from "@/components/ui/separator";
import { api, ApiError } from "@/lib/api";
import { useAuthStore } from "@/stores/auth";

const profileSchema = z.object({
  username: z.string().min(1, "Username is required").max(50),
  bio: z.string().max(500, "Bio must be 500 characters or less").optional(),
  website: z.string().url("Invalid URL").or(z.literal("")).optional(),
  avatar: z.string().url("Invalid URL").or(z.literal("")).optional(),
});

type ProfileForm = z.infer<typeof profileSchema>;

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

export default function ProfilePage() {
  const router = useRouter();
  const { user, setUser, logout } = useAuthStore();
  const [profileLoading, setProfileLoading] = useState(false);
  const [passwordLoading, setPasswordLoading] = useState(false);

  const {
    register: regProfile,
    handleSubmit: handleProfileSubmit,
    formState: { errors: profileErrors },
  } = useForm<ProfileForm>({
    resolver: zodResolver(profileSchema as never),
    defaultValues: {
      username: user?.username ?? "",
      bio: "",
      website: "",
      avatar: "",
    },
  });

  const {
    register: regPassword,
    handleSubmit: handlePasswordSubmit,
    reset: resetPassword,
    formState: { errors: passwordErrors },
  } = useForm<PasswordForm>({
    resolver: zodResolver(passwordSchema as never),
  });

  async function onProfileSubmit(values: ProfileForm) {
    setProfileLoading(true);
    try {
      const updated = await api.put<{
        id: string;
        username: string;
        email: string;
        role: string;
        avatar: string | null;
        bio: string | null;
      }>("/users/me", {
        username: values.username,
        bio: values.bio || undefined,
        website: values.website || undefined,
        avatar: values.avatar || undefined,
      });
      setUser(updated);
      toast.success("Profile updated");
    } catch (err) {
      if (err instanceof ApiError) {
        toast.error(err.message);
      } else {
        toast.error("Failed to update profile");
      }
    } finally {
      setProfileLoading(false);
    }
  }

  async function onPasswordSubmit(values: PasswordForm) {
    setPasswordLoading(true);
    try {
      await api.put("/users/me/password", {
        old_password: values.old_password,
        new_password: values.new_password,
      });
      toast.success("Password changed. Please log in again.");
      logout();
      router.push("/auth/login");
    } catch (err) {
      if (err instanceof ApiError) {
        toast.error(err.message);
      } else {
        toast.error("Failed to change password");
      }
    } finally {
      setPasswordLoading(false);
    }
  }

  return (
    <div className="mx-auto max-w-2xl space-y-6 py-8">
      <h1 className="text-2xl font-bold">Profile Settings</h1>

      <Card>
        <CardHeader>
          <CardTitle>Account Info</CardTitle>
          <CardDescription>
            {user?.email} &middot; {user?.role}
          </CardDescription>
        </CardHeader>
        <CardContent>
          <form
            onSubmit={handleProfileSubmit(onProfileSubmit)}
            className="space-y-4"
          >
            <div className="space-y-2">
              <Label htmlFor="username">Username</Label>
              <Input id="username" {...regProfile("username")} />
              {profileErrors.username && (
                <p className="text-sm text-red-500">
                  {profileErrors.username.message}
                </p>
              )}
            </div>

            <div className="space-y-2">
              <Label htmlFor="bio">Bio</Label>
              <Textarea
                id="bio"
                placeholder="Tell us about yourself"
                rows={3}
                {...regProfile("bio")}
              />
              {profileErrors.bio && (
                <p className="text-sm text-red-500">
                  {profileErrors.bio.message}
                </p>
              )}
            </div>

            <div className="space-y-2">
              <Label htmlFor="website">Website</Label>
              <Input
                id="website"
                placeholder="https://example.com"
                {...regProfile("website")}
              />
              {profileErrors.website && (
                <p className="text-sm text-red-500">
                  {profileErrors.website.message}
                </p>
              )}
            </div>

            <div className="space-y-2">
              <Label htmlFor="avatar">Avatar URL</Label>
              <Input
                id="avatar"
                placeholder="https://example.com/avatar.jpg"
                {...regProfile("avatar")}
              />
              {profileErrors.avatar && (
                <p className="text-sm text-red-500">
                  {profileErrors.avatar.message}
                </p>
              )}
            </div>

            <Button type="submit" disabled={profileLoading}>
              {profileLoading ? "Saving..." : "Save Profile"}
            </Button>
          </form>
        </CardContent>
      </Card>

      <Separator />

      <Card>
        <CardHeader>
          <CardTitle>Change Password</CardTitle>
          <CardDescription>
            After changing your password, you will be logged out and need to
            sign in again.
          </CardDescription>
        </CardHeader>
        <CardContent>
          <form
            onSubmit={handlePasswordSubmit(onPasswordSubmit)}
            className="space-y-4"
          >
            <div className="space-y-2">
              <Label htmlFor="old_password">Current Password</Label>
              <Input
                id="old_password"
                type="password"
                {...regPassword("old_password")}
              />
              {passwordErrors.old_password && (
                <p className="text-sm text-red-500">
                  {passwordErrors.old_password.message}
                </p>
              )}
            </div>

            <div className="space-y-2">
              <Label htmlFor="new_password">New Password</Label>
              <Input
                id="new_password"
                type="password"
                {...regPassword("new_password")}
              />
              {passwordErrors.new_password && (
                <p className="text-sm text-red-500">
                  {passwordErrors.new_password.message}
                </p>
              )}
            </div>

            <div className="space-y-2">
              <Label htmlFor="confirm_password">Confirm New Password</Label>
              <Input
                id="confirm_password"
                type="password"
                {...regPassword("confirm_password")}
              />
              {passwordErrors.confirm_password && (
                <p className="text-sm text-red-500">
                  {passwordErrors.confirm_password.message}
                </p>
              )}
            </div>

            <Button type="submit" variant="destructive" disabled={passwordLoading}>
              {passwordLoading ? "Changing..." : "Change Password"}
            </Button>
          </form>
        </CardContent>
      </Card>
    </div>
  );
}
