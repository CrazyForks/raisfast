"use client";

import { useRouter } from "next/navigation";
import { LogOut, User, LayoutDashboard, Package } from "lucide-react";
import {
  Avatar,
  AvatarImage,
  AvatarFallback,
} from "@/components/ui/avatar";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { useAuthStore } from "@/stores/auth";
import { client } from "@/lib/raisfast";

interface UserMenuProps {
  onAction?: () => void;
}

export function UserMenu({ onAction }: UserMenuProps) {
  const router = useRouter();
  const user = useAuthStore((s) => s.user);
  const logout = useAuthStore((s) => s.logout);
  const isAuthor = useAuthStore((s) => s.isAuthor());

  async function handleLogout() {
    try {
      await client.auth.logout();
    } catch {
      // ignore errors on logout
    }
    logout();
    onAction?.();
    router.push("/");
  }

  if (!user) return null;

  return (
    <DropdownMenu>
      <DropdownMenuTrigger className="rounded-full outline-none focus-visible:ring-2 focus-visible:ring-ring">
        <Avatar size="sm">
          {user.avatar && <AvatarImage src={user.avatar} alt={user.username} />}
          <AvatarFallback>{user.username.charAt(0).toUpperCase()}</AvatarFallback>
        </Avatar>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="end">
        <div className="px-1.5 py-1.5">
          <p className="text-sm font-medium">{user.username}</p>
          <p className="text-xs text-muted-foreground">{user.email}</p>
        </div>
        <DropdownMenuSeparator />
        <DropdownMenuItem
          className="cursor-pointer"
          onClick={() => {
            onAction?.();
            router.push("/profile");
          }}
        >
          <User />
          Profile
        </DropdownMenuItem>
        <DropdownMenuItem
          className="cursor-pointer"
          onClick={() => {
            onAction?.();
            router.push("/orders");
          }}
        >
          <Package />
          My Orders
        </DropdownMenuItem>
        {isAuthor && (
          <DropdownMenuItem
            className="cursor-pointer"
            onClick={() => {
              onAction?.();
              router.push("/admin/dashboard");
            }}
          >
            <LayoutDashboard />
            Dashboard
          </DropdownMenuItem>
        )}
        <DropdownMenuSeparator />
        <DropdownMenuItem
          className="cursor-pointer"
          variant="destructive"
          onClick={handleLogout}
        >
          <LogOut />
          Logout
        </DropdownMenuItem>
      </DropdownMenuContent>
    </DropdownMenu>
  );
}
