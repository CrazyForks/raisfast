
import { useEffect, useState } from "react";
import Link from "@/lib/link";
import { usePathname, useRouter } from "@/lib/navigation";
import { Outlet, Navigate } from "react-router-dom";

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
  Puzzle,
  Moon,
  Sun,
  KeyRound,
  GitBranch,
  Languages,
  LayoutTemplate,
  Blocks,
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
import { useTheme } from "next-themes";
import { useT, useI18nStore, type Locale } from "@/lib/i18n";

function useContentItems() {
  const { t } = useT();
  return [
    { label: t("layout.posts"), href: "/posts", icon: FileText },
    { label: t("layout.categories"), href: "/categories", icon: Folder },
    { label: t("layout.tags"), href: "/tags", icon: Tag },
    { label: t("layout.comments"), href: "/comments", icon: MessageSquare },
    { label: t("layout.media"), href: "/media", icon: Image },
    { label: t("layout.pages"), href: "/pages", icon: LayoutTemplate },
    { label: t("layout.reusableBlocks"), href: "/reusable-blocks", icon: Blocks },
    { label: t("layout.contentTypes"), href: "/content-types", icon: Layers },
  ];
}

function useSystemItems() {
  const { t } = useT();
  const { builtinTenantable } = useTenantStore();
  return [
    { label: t("layout.users"), href: "/users", icon: Users },
    { label: t("layout.plugins"), href: "/plugins", icon: Puzzle },
    { label: t("layout.rolesPermissions"), href: "/rbac", icon: ShieldCheck },
    { label: t("layout.cron"), href: "/crons", icon: Clock },
    ...(builtinTenantable ? [{ label: t("layout.tenants"), href: "/tenants", icon: Building2 }] : []),
    { label: t("layout.webhooks"), href: "/webhooks", icon: Webhook },
    { label: t("layout.apiTokens"), href: "/tokens", icon: KeyRound },
    { label: t("layout.workflows"), href: "/workflows", icon: GitBranch },
    { label: t("layout.auditLog"), href: "/audit", icon: ClipboardList },
    { label: t("layout.options"), href: "/options", icon: Settings },
  ];
}

function TenantSwitcher() {
  const { t } = useT();
  const { currentTenantId, clearTenant, builtinTenantable } = useTenantStore();
  const { isAdmin } = useAuthStore();

  if (!builtinTenantable || !isAdmin()) return null;

  return (
    <div className="flex items-center gap-2 px-4 py-2">
      <Globe className="size-3.5 text-muted-foreground shrink-0" />
      <span className="text-[11px] text-muted-foreground uppercase tracking-wider font-medium">
        {t("layout.tenant")}
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
            title={t("layout.clearTenant")}
          >
            <X className="size-3" />
          </Button>
        </div>
      ) : (
        <span className="text-xs text-muted-foreground/60 italic">{t("layout.all")}</span>
      )}
    </div>
  );
}

function LanguageToggle() {
  const { locale, setLocale } = useI18nStore();

  return (
    <Button
      variant="ghost"
      size="icon-sm"
      onClick={() => setLocale(locale === "en" ? "zh" : "en")}
      title={locale === "en" ? "切换中文" : "Switch to English"}
    >
      <Languages className="size-4" />
    </Button>
  );
}

export default function AdminLayout() {
  const pathname = usePathname();
  const { isLoggedIn, isAuthor, logout, user } = useAuthStore();
  const { resolvedTheme, setTheme } = useTheme();
  const [mounted, setMounted] = useState(false);
  const contentItems = useContentItems();
  const systemItems = useSystemItems();
  const { t } = useT();
  const { setBuiltinTenantable } = useTenantStore();

  useEffect(() => {
    setMounted(true);
    fetch("/api/v1/options/public")
      .then((r) => r.json())
      .then((body) => {
        if (body.code === 0 && body.data?.builtin_tenantable != null) {
          setBuiltinTenantable(body.data.builtin_tenantable);
        }
      })
      .catch(() => {});
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
    return <Navigate to="/auth/login" replace />;
  }

  function getIsActive(href: string) {
    return pathname === href || (href !== "/dashboard" && pathname.startsWith(href));
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
              <span className="text-sm font-semibold leading-tight">{t("layout.brand")}</span>
              <span className="text-[11px] text-muted-foreground leading-tight">{t("layout.adminPanel")}</span>
            </div>
          </div>
        </SidebarHeader>

        <SidebarSeparator />
        <TenantSwitcher />
        <SidebarSeparator />

        <SidebarContent>
          <SidebarGroup>
            <SidebarGroupContent>
              <SidebarMenu>
                <SidebarMenuItem>
                  <SidebarMenuButton
                    render={<Link href="/dashboard" />}
                    isActive={getIsActive("/dashboard")}
                    tooltip={t("layout.dashboard")}
                  >
                    <LayoutDashboard />
                    <span>{t("layout.dashboard")}</span>
                  </SidebarMenuButton>
                </SidebarMenuItem>
              </SidebarMenu>
            </SidebarGroupContent>
          </SidebarGroup>

          <SidebarSeparator />

          <SidebarGroup>
            <SidebarGroupLabel>
              <ChevronDown className="size-3" />
              {t("layout.content")}
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

          <SidebarGroup>
            <SidebarGroupLabel>
              <ChevronDown className="size-3" />
              {t("layout.system")}
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
            <LanguageToggle />
            <Button
              variant="ghost"
              size="icon-sm"
              onClick={() => setTheme(resolvedTheme === "dark" ? "light" : "dark")}
              title={t("layout.toggleTheme")}
            >
              {mounted && resolvedTheme === "dark" ? (
                <Sun className="size-4" />
              ) : (
                <Moon className="size-4" />
              )}
            </Button>
            <Button
              variant="ghost"
              size="icon-sm"
              onClick={logout}
              title={t("layout.signOut")}
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
        <div className="flex-1 p-6"><Outlet /></div>
      </SidebarInset>
    </SidebarProvider>
  );
}
