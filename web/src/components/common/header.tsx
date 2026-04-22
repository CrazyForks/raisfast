"use client";

import Link from "next/link";
import { useEffect, useState } from "react";
import { useRouter } from "next/navigation";
import { Menu, Rss, Search, ShoppingCart } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { Input } from "@/components/ui/input";
import {
  Sheet,
  SheetContent,
  SheetHeader,
  SheetTitle,
  SheetTrigger,
} from "@/components/ui/sheet";
import { useAuthStore } from "@/stores/auth";
import { useCartStore } from "@/stores/cart";
import { UserMenu } from "@/components/common/user-menu";
import { ThemeToggle } from "@/components/common/theme-toggle";

const navLinks = [
  { href: "/posts", label: "Posts" },
  { href: "/forum", label: "Forum" },
  { href: "/shop", label: "Shop" },
  {
    href: "/feed.xml",
    label: "RSS",
    external: true,
  },
];

export function Header() {
  const [open, setOpen] = useState(false);
  const [mounted, setMounted] = useState(false);
  const [searchOpen, setSearchOpen] = useState(false);
  const [searchValue, setSearchValue] = useState("");
  const router = useRouter();
  const isLoggedIn = useAuthStore((s) => s.isLoggedIn());
  const cartItemCount = useCartStore((s) => s.itemCount());
  const fetchCart = useCartStore((s) => s.fetchCart);

  useEffect(() => {
    setMounted(true);
    fetchCart();
  }, [fetchCart]);

  function handleSearch(e: React.FormEvent) {
    e.preventDefault();
    if (searchValue.trim()) {
      router.push(`/posts?q=${encodeURIComponent(searchValue.trim())}`);
      setSearchOpen(false);
      setSearchValue("");
    }
  }

  return (
    <header className="sticky top-0 z-40 w-full border-b bg-background/95 backdrop-blur-sm">
      <div className="mx-auto flex h-14 max-w-5xl items-center justify-between px-4">
        <Link
          href="/"
          className="text-lg font-bold tracking-tight hover:opacity-80"
        >
          Blog
        </Link>

        <nav className="hidden items-center gap-1 md:flex">
          {navLinks.map((link) =>
            link.external ? (
              <a
                key={link.href}
                href={link.href}
                target="_blank"
                rel="noopener noreferrer"
                className="inline-flex items-center gap-1 rounded-md px-3 py-1.5 text-sm text-muted-foreground hover:bg-muted hover:text-foreground"
              >
                {link.label}
                <Rss className="size-3" />
              </a>
            ) : (
              <Link
                key={link.href}
                href={link.href}
                className="rounded-md px-3 py-1.5 text-sm text-muted-foreground hover:bg-muted hover:text-foreground"
              >
                {link.label}
              </Link>
            ),
          )}
        </nav>

        <div className="hidden items-center gap-2 md:flex">
          {searchOpen ? (
            <form onSubmit={handleSearch} className="flex items-center">
              <Input
                type="text"
                value={searchValue}
                onChange={(e) => setSearchValue(e.target.value)}
                placeholder="Search..."
                className="h-8 w-48 text-sm"
                autoFocus
                onBlur={() => { if (!searchValue) setSearchOpen(false); }}
              />
            </form>
          ) : (
            <Button
              variant="ghost"
              size="icon-sm"
              aria-label="Search"
              onClick={() => setSearchOpen(true)}
            >
              <Search className="h-4 w-4" />
            </Button>
          )}
        </div>

        <div className="hidden items-center gap-2 md:flex">
          <ThemeToggle />
          {mounted && (
            <Button
              variant="ghost"
              size="icon-sm"
              aria-label="Cart"
              className="relative"
              onClick={() => router.push("/cart")}
            >
              <ShoppingCart className="h-4 w-4" />
              {cartItemCount > 0 && (
                <span className="absolute -right-1 -top-1 flex h-4 w-4 items-center justify-center rounded-full bg-primary text-[10px] font-bold text-primary-foreground">
                  {cartItemCount > 9 ? "9+" : cartItemCount}
                </span>
              )}
            </Button>
          )}
        </div>

        <div className="hidden items-center gap-2 md:flex">
          {!mounted ? null : isLoggedIn ? (
            <UserMenu />
          ) : (
            <>
              <Link href="/auth/login">
                <Button variant="ghost" size="sm">Login</Button>
              </Link>
              <Link href="/auth/register">
                <Button size="sm">Register</Button>
              </Link>
            </>
          )}
        </div>

        <div className="md:hidden">
          <Sheet open={open} onOpenChange={setOpen}>
            <SheetTrigger
              render={
                <Button variant="ghost" size="icon-sm" aria-label="Menu" />
              }
            >
              <Menu />
            </SheetTrigger>
            <SheetContent side="right">
              <SheetHeader>
                <SheetTitle>Menu</SheetTitle>
              </SheetHeader>
              <nav className="flex flex-col gap-1 px-4">
                <Link
                  href="/posts"
                  className="flex items-center gap-2 rounded-md px-3 py-2 text-sm text-muted-foreground hover:bg-muted hover:text-foreground"
                  onClick={() => setOpen(false)}
                >
                  <Search className="size-4" />
                  Search
                </Link>
                {navLinks.map((link) =>
                  link.external ? (
                    <a
                      key={link.href}
                      href={link.href}
                      target="_blank"
                      rel="noopener noreferrer"
                      className="flex items-center gap-2 rounded-md px-3 py-2 text-sm text-muted-foreground hover:bg-muted hover:text-foreground"
                      onClick={() => setOpen(false)}
                    >
                      {link.label}
                      <Rss className="size-3" />
                    </a>
                  ) : (
                    <Link
                      key={link.href}
                      href={link.href}
                      className="rounded-md px-3 py-2 text-sm text-muted-foreground hover:bg-muted hover:text-foreground"
                      onClick={() => setOpen(false)}
                    >
                      {link.label}
                    </Link>
                  ),
                )}
                <div className="my-2 h-px bg-border" />
                {mounted && isLoggedIn && (
                  <Link
                    href="/cart"
                    className="flex items-center gap-2 rounded-md px-3 py-2 text-sm text-muted-foreground hover:bg-muted hover:text-foreground"
                    onClick={() => setOpen(false)}
                  >
                    <ShoppingCart className="size-4" />
                    Cart
                    {cartItemCount > 0 && (
                      <Badge variant="default" className="ml-auto text-[10px] px-1.5 py-0">
                        {cartItemCount}
                      </Badge>
                    )}
                  </Link>
                )}
                <div className="px-3 py-2">
                  <ThemeToggle />
                </div>
                <div className="my-2 h-px bg-border" />
                {!mounted ? null : isLoggedIn ? (
                  <div className="px-3 py-2">
                    <UserMenu onAction={() => setOpen(false)} />
                  </div>
                ) : (
                  <div className="flex flex-col gap-2 px-3">
                    <Link href="/auth/login" onClick={() => setOpen(false)}>
                      <Button variant="outline" className="w-full">Login</Button>
                    </Link>
                    <Link href="/auth/register" onClick={() => setOpen(false)}>
                      <Button className="w-full">Register</Button>
                    </Link>
                  </div>
                )}
              </nav>
            </SheetContent>
          </Sheet>
        </div>
      </div>
    </header>
  );
}
