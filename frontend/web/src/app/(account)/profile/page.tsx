"use client";

import { useState } from "react";
import { useForm } from "react-hook-form";
import { z } from "zod";
import { zodResolver } from "@hookform/resolvers/zod";
import { toast } from "sonner";

import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle, CardDescription } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Textarea } from "@/components/ui/textarea";
import { client } from "@/lib/raisfast";
import { SDKError } from "@raisfast/sdk";
import { useAuthStore } from "@/stores/auth";

const profileSchema = z.object({
  username: z.string().min(1, "Username is required").max(50),
  bio: z.string().max(500, "Bio must be 500 characters or less").optional(),
  website: z.string().url("Invalid URL").or(z.literal("")).optional(),
  avatar: z.string().url("Invalid URL").or(z.literal("")).optional(),
});

type ProfileForm = z.infer<typeof profileSchema>;

export default function ProfilePage() {
  const { user, setUser } = useAuthStore();
  const [loading, setLoading] = useState(false);

  const {
    register,
    handleSubmit,
    formState: { errors },
  } = useForm<ProfileForm>({
    resolver: zodResolver(profileSchema as never),
    defaultValues: {
      username: user?.username ?? "",
      bio: "",
      website: "",
      avatar: "",
    },
  });

  async function onSubmit(values: ProfileForm) {
    setLoading(true);
    try {
      const updated = await client.users.updateMe({
        nickname: values.username,
        avatar: values.avatar || undefined,
      });
      setUser({
        id: updated.id,
        email: updated.email,
        username: updated.nickname,
        role: updated.role,
        avatar: updated.avatar,
        bio: null,
      });
      toast.success("Profile updated");
    } catch (err) {
      if (err instanceof SDKError) {
        toast.error(err.message);
      } else {
        toast.error("Failed to update profile");
      }
    } finally {
      setLoading(false);
    }
  }

  return (
    <div className="space-y-6">
      <div>
        <h1 className="text-2xl font-bold">Profile</h1>
        <p className="text-muted-foreground">Manage your account information.</p>
      </div>

      <Card>
        <CardHeader>
          <CardTitle>Account Info</CardTitle>
          <CardDescription>
            {user?.email} &middot; {user?.role}
          </CardDescription>
        </CardHeader>
        <CardContent>
          <form onSubmit={handleSubmit(onSubmit)} className="space-y-4">
            <div className="space-y-2">
              <Label htmlFor="username">Username</Label>
              <Input id="username" {...register("username")} />
              {errors.username && <p className="text-sm text-red-500">{errors.username.message}</p>}
            </div>

            <div className="space-y-2">
              <Label htmlFor="bio">Bio</Label>
              <Textarea id="bio" placeholder="Tell us about yourself" rows={3} {...register("bio")} />
              {errors.bio && <p className="text-sm text-red-500">{errors.bio.message}</p>}
            </div>

            <div className="space-y-2">
              <Label htmlFor="website">Website</Label>
              <Input id="website" placeholder="https://example.com" {...register("website")} />
              {errors.website && <p className="text-sm text-red-500">{errors.website.message}</p>}
            </div>

            <div className="space-y-2">
              <Label htmlFor="avatar">Avatar URL</Label>
              <Input id="avatar" placeholder="https://example.com/avatar.jpg" {...register("avatar")} />
              {errors.avatar && <p className="text-sm text-red-500">{errors.avatar.message}</p>}
            </div>

            <Button type="submit" disabled={loading}>
              {loading ? "Saving..." : "Save Profile"}
            </Button>
          </form>
        </CardContent>
      </Card>
    </div>
  );
}
