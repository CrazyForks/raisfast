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
  Puzzle,
  Moon,
  Sun,
  KeyRound,
  GitBranch,
  Languages,
  Handshake,
  Kanban,
  Contact,
  Building,
  FileBarChart,
  StickyNote,
  CalendarCheck,
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
    { label: t("layout.posts"), href: "/admin/posts", icon: FileText },
    { label: t("layout.categories"), href: "/admin/categories", icon: Folder },
    { label: t("layout.tags"), href: "/admin/tags", icon: Tag },
    { label: t("layout.comments"), href: "/admin/comments", icon: MessageSquare },
    { label: t("layout.media"), href: "/admin/media", icon: Image },
    { label: t("layout.pages"), href: "/admin/pages", icon: LayoutTemplate },
    { label: t("layout.reusableBlocks"), href: "/admin/reusable-blocks", icon: Blocks },
    { label: t("layout.contentTypes"), href: "/admin/content-types", icon: Layers },
  ];
}

function useCrmItems() {
  const { t } = useT();
  return [
    { label: t("crm.dashboard"), href: "/admin/crm/dashboard", icon: Handshake },
    { label: t("crm.pipeline"), href: "/admin/crm/pipeline", icon: Kanban },
    { label: t("crm.contacts"), href: "/admin/crm/contacts", icon: Contact },
    { label: t("crm.companies"), href: "/admin/crm/companies", icon: Building },
    { label: t("crm.deals"), href: "/admin/crm/deals", icon: FileText },
    { label: t("crm.activities"), href: "/admin/crm/activities", icon: CalendarCheck },
    { label: t("crm.notes"), href: "/admin/crm/notes", icon: StickyNote },
    { label: t("crm.reports"), href: "/admin/crm/reports", icon: FileBarChart },
  ];
}

function useSystemItems() {
  const { t } = useT();
  return [
    { label: t("layout.users"), href: "/admin/users", icon: Users },
    { label: t("layout.plugins"), href: "/admin/plugins", icon: Puzzle },
    { label: t("layout.rolesPermissions"), href: "/admin/rbac", icon: ShieldCheck },
    { label: t("layout.cron"), href: "/admin/crons", icon: Clock },
    { label: t("layout.tenants"), href: "/admin/tenants", icon: Building2 },
    { label: t("layout.webhooks"), href: "/admin/webhooks", icon: Webhook },
    { label: t("layout.apiTokens"), href: "/admin/tokens", icon: KeyRound },
    { label: t("layout.workflows"), href: "/admin/workflows", icon: GitBranch },
    { label: t("layout.auditLog"), href: "/admin/audit", icon: ClipboardList },
    { label: t("layout.options"), href: "/admin/options", icon: Settings },
  ];
}

function TenantSwitcher() {
  const { t } = useT();
  const { currentTenantId, clearTenant } = useTenantStore();
  const { isAdmin } = useAuthStore();

  if (!isAdmin()) return null;

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

export default function AdminLayout({
  children,
}: {
  children: React.ReactNode;
}) {
  const pathname = usePathname();
  const { isLoggedIn, isAuthor, logout, user } = useAuthStore();
  const { resolvedTheme, setTheme } = useTheme();
  const [mounted, setMounted] = useState(false);
  const contentItems = useContentItems();
  const crmItems = useCrmItems();
  const systemItems = useSystemItems();
  const { t } = useT();

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
          <h1 className="text-2xl font-bold">{t("layout.accessDenied")}</h1>
          <p className="text-muted-foreground">
            {t("layout.accessDeniedMsg")}
          </p>
          <div>
            <Link href="/auth/login">
              <Button>{t("layout.goToLogin")}</Button>
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
                    render={<Link href="/admin/dashboard" />}
                    isActive={getIsActive("/admin/dashboard")}
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
              {t("layout.crm")}
            </SidebarGroupLabel>
            <SidebarGroupContent>
              <SidebarMenu>
                {crmItems.map((item) => {
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
        <div className="flex-1 p-6">{children}</div>
      </SidebarInset>
    </SidebarProvider>
  );
}
