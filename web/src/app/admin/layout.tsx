"use client";

import { useEffect, useState } from "react";
import Link from "next/link";
import { usePathname } from "next/navigation";

import {
  LayoutDashboard,
  FileText,
  Folder,
  Tag,
  MessageSquare,
  Image,
  Users,
  Clock,
  LogOut,
  Settings,
  Layers,
  Building2,
  ClipboardList,
  Globe,
  Webhook,
  X,
  ShieldCheck,
  ChevronDown,
  PenLine,
  Package,
} from "lucide-react";

import {
  SidebarProvider,
  Sidebar,
  SidebarContent,
  SidebarGroup,
  SidebarGroupContent,
  SidebarGroupLabel,
  SidebarHeader,
  SidebarMenu,
  SidebarMenuItem,
  SidebarMenuButton,
  SidebarFooter,
  SidebarInset,
  SidebarTrigger,
  SidebarSeparator,
} from "@/components/ui/sidebar";
import { Button } from "@/components/ui/button";
import { useAuthStore } from "@/stores/auth";
import { useTenantStore } from "@/stores/tenant";

const contentItems = [
  { label: "Posts", href: "/admin/posts", icon: FileText },
  { label: "Categories", href: "/admin/categories", icon: Folder },
  { label: "Tags", href: "/admin/tags", icon: Tag },
  { label: "Comments", href: "/admin/comments", icon: MessageSquare },
  { label: "Media", href: "/admin/media", icon: Image },
  { label: "Content Types", href: "/admin/content-types", icon: Layers },
];

const systemItems = [
  { label: "Users", href: "/admin/users", icon: Users },
  { label: "Extensions", href: "/admin/extensions", icon: Package },
  { label: "Roles & Permissions", href: "/admin/rbac", icon: ShieldCheck },
  { label: "Cron", href: "/admin/crons", icon: Clock },
  { label: "Tenants", href: "/admin/tenants", icon: Building2 },
  { label: "Webhooks", href: "/admin/webhooks", icon: Webhook },
  { label: "Audit Log", href: "/admin/audit", icon: ClipboardList },
  { label: "Options", href: "/admin/options", icon: Settings },
];

function TenantSwitcher() {
  const { currentTenantId, setTenant, clearTenant } = useTenantStore();
  const { isAdmin } = useAuthStore();

  if (!isAdmin()) return null;

  return (
    <div className="flex items-center gap-2 px-4 py-2">
      <Globe className="size-3.5 text-muted-foreground shrink-0" />
      <span className="text-[11px] text-muted-foreground uppercase tracking-wider font-medium">
        Tenant
      </span>
      <div className="flex-1 min-w-0" />
      {currentTenantId ? (
        <div className="flex items-center gap-1">
          <span className="text-xs font-medium truncate max-w-[80px]">
            {currentTenantId}
          </span>
          <Button
            variant="ghost"
            size="icon-sm"
            onClick={clearTenant}
            title="Clear tenant filter"
          >
            <X className="size-3" />
          </Button>
        </div>
      ) : (
        <span className="text-xs text-muted-foreground/60 italic">All</span>
      )}
    </div>
  );
}

export default function AdminLayout({
  children,
}: {
  children: React.ReactNode;
}) {
  const pathname = usePathname();
  const { isLoggedIn, isAuthor, logout, user } = useAuthStore();
  const [mounted, setMounted] = useState(false);

  useEffect(() => {
    setMounted(true);
  }, []);

  if (!mounted) {
    return (
      <div className="flex min-h-screen items-center justify-center">
        <div className="text-center space-y-4">
          <div className="size-8 animate-spin rounded-full border-2 border-muted border-t-transparent mx-auto" />
        </div>
      </div>
    );
  }

  if (!isLoggedIn() || !isAuthor()) {
    return (
      <div className="flex min-h-screen items-center justify-center">
        <div className="text-center space-y-4">
          <h1 className="text-2xl font-bold">Access Denied</h1>
          <p className="text-muted-foreground">
            You need to be logged in as an author or admin to access this area.
          </p>
          <div>
            <Link href="/auth/login">
              <Button>Go to Login</Button>
            </Link>
          </div>
        </div>
      </div>
    );
  }

  function getIsActive(href: string) {
    return pathname === href || (href !== "/admin/dashboard" && pathname.startsWith(href));
  }

  return (
    <SidebarProvider>
      <Sidebar>
        <SidebarHeader className="px-4 py-5">
          <div className="flex items-center gap-2.5">
            <div className="flex size-8 items-center justify-center rounded-lg bg-primary text-primary-foreground">
              <PenLine className="size-4" />
            </div>
            <div className="flex flex-col">
              <span className="text-sm font-semibold leading-tight">Rust Blog</span>
              <span className="text-[11px] text-muted-foreground leading-tight">Admin Panel</span>
            </div>
          </div>
        </SidebarHeader>

        <SidebarSeparator />
        <TenantSwitcher />
        <SidebarSeparator />

        <SidebarContent>
          {/* Dashboard — standalone */}
          <SidebarGroup>
            <SidebarGroupContent>
              <SidebarMenu>
                <SidebarMenuItem>
                  <SidebarMenuButton
                    render={<Link href="/admin/dashboard" />}
                    isActive={getIsActive("/admin/dashboard")}
                    tooltip="Dashboard"
                  >
                    <LayoutDashboard />
                    <span>Dashboard</span>
                  </SidebarMenuButton>
                </SidebarMenuItem>
              </SidebarMenu>
            </SidebarGroupContent>
          </SidebarGroup>

          <SidebarSeparator />

          {/* Content section */}
          <SidebarGroup>
            <SidebarGroupLabel>
              <ChevronDown className="size-3" />
              Content
            </SidebarGroupLabel>
            <SidebarGroupContent>
              <SidebarMenu>
                {contentItems.map((item) => {
                  const active = getIsActive(item.href);
                  return (
                    <SidebarMenuItem key={item.href}>
                      <SidebarMenuButton
                        render={<Link href={item.href} />}
                        isActive={active}
                        tooltip={item.label}
                      >
                        <item.icon />
                        <span>{item.label}</span>
                      </SidebarMenuButton>
                    </SidebarMenuItem>
                  );
                })}
              </SidebarMenu>
            </SidebarGroupContent>
          </SidebarGroup>

          <SidebarSeparator />

          {/* System section */}
          <SidebarGroup>
            <SidebarGroupLabel>
              <ChevronDown className="size-3" />
              System
            </SidebarGroupLabel>
            <SidebarGroupContent>
              <SidebarMenu>
                {systemItems.map((item) => {
                  const active = getIsActive(item.href);
                  return (
                    <SidebarMenuItem key={item.href}>
                      <SidebarMenuButton
                        render={<Link href={item.href} />}
                        isActive={active}
                        tooltip={item.label}
                      >
                        <item.icon />
                        <span>{item.label}</span>
                      </SidebarMenuButton>
                    </SidebarMenuItem>
                  );
                })}
              </SidebarMenu>
            </SidebarGroupContent>
          </SidebarGroup>
        </SidebarContent>

        <SidebarFooter className="px-3 py-3">
          <SidebarSeparator className="mb-2" />
          <div className="flex items-center gap-2.5 rounded-md px-2 py-1.5">
            <div className="flex size-7 items-center justify-center rounded-full bg-muted text-xs font-medium">
              {user?.username?.charAt(0).toUpperCase() ?? "U"}
            </div>
            <div className="flex-1 min-w-0">
              <p className="text-sm font-medium truncate leading-tight">
                {user?.username}
              </p>
              <p className="text-[11px] text-muted-foreground truncate leading-tight">
                {user?.role}
              </p>
            </div>
            <Button
              variant="ghost"
              size="icon-sm"
              onClick={logout}
              title="Sign out"
            >
              <LogOut className="size-4" />
            </Button>
          </div>
        </SidebarFooter>
      </Sidebar>

      <SidebarInset>
        <header className="flex h-12 items-center gap-2 border-b px-4">
          <SidebarTrigger />
        </header>
        <div className="flex-1 p-6">{children}</div>
      </SidebarInset>
    </SidebarProvider>
  );
}
