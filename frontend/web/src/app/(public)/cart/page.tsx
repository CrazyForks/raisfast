"use client";

import { useEffect } from "react";
import Link from "next/link";
import { useRouter } from "next/navigation";
import {
  ArrowLeft,
  Minus,
  Plus,
  ShoppingCart,
  Trash2,
  Package,
} from "lucide-react";
import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import { Separator } from "@/components/ui/separator";
import { useCartStore } from "@/stores/cart";
import { useAuthStore } from "@/stores/auth";
import { toast } from "sonner";

function formatPrice(price: number) {
  return new Intl.NumberFormat("en-US", {
    style: "currency",
    currency: "USD",
  }).format(price);
}

export default function CartPage() {
  const router = useRouter();
  const items = useCartStore((s) => s.items);
  const total = useCartStore((s) => s.total);
  const loading = useCartStore((s) => s.loading);
  const fetchCart = useCartStore((s) => s.fetchCart);
  const updateItem = useCartStore((s) => s.updateItem);
  const removeItem = useCartStore((s) => s.removeItem);
  const clearAll = useCartStore((s) => s.clearAll);
  const isLoggedIn = useAuthStore((s) => s.isLoggedIn());

  useEffect(() => {
    if (!isLoggedIn) {
      router.push("/auth/login");
      return;
    }
    fetchCart();
  }, [isLoggedIn, fetchCart, router]);

  async function handleUpdateQuantity(itemId: string, qty: number) {
    try {
      await updateItem(itemId, qty);
    } catch (err) {
      toast.error(err instanceof Error ? err.message : "Failed to update");
    }
  }

  async function handleRemove(itemId: string) {
    try {
      await removeItem(itemId);
      toast.success("Item removed");
    } catch (err) {
      toast.error(err instanceof Error ? err.message : "Failed to remove");
    }
  }

  async function handleClear() {
    try {
      await clearAll();
      toast.success("Cart cleared");
    } catch (err) {
      toast.error(err instanceof Error ? err.message : "Failed to clear");
    }
  }

  if (!isLoggedIn) return null;

  return (
    <div className="space-y-6">
      <div className="flex items-center justify-between">
        <div className="flex items-center gap-3">
          <Link
            href="/shop"
            className="inline-flex items-center gap-1 text-sm text-muted-foreground hover:text-foreground"
          >
            <ArrowLeft className="h-4 w-4" />
            Continue Shopping
          </Link>
        </div>
        <h1 className="text-2xl font-bold">Shopping Cart</h1>
      </div>

      {loading && items.length === 0 ? (
        <div className="flex justify-center py-16">
          <div className="h-8 w-8 animate-spin rounded-full border-4 border-muted-foreground border-t-transparent" />
        </div>
      ) : items.length === 0 ? (
        <div className="flex flex-col items-center justify-center py-16 text-muted-foreground">
          <ShoppingCart className="h-12 w-12 mb-4" />
          <p className="text-lg font-medium">Your cart is empty</p>
          <p className="text-sm mb-4">Add some products to get started</p>
          <Link href="/shop">
            <Button>Browse Products</Button>
          </Link>
        </div>
      ) : (
        <div className="grid gap-6 lg:grid-cols-3">
          <div className="lg:col-span-2 space-y-4">
            <div className="flex justify-end">
              <Button variant="ghost" size="sm" onClick={handleClear}>
                <Trash2 className="h-3.5 w-3.5" />
                Clear Cart
              </Button>
            </div>
            {items.map((item) => {
              const images = item.images
                ? item.images.split(",").filter(Boolean)
                : [];
              const img = images[0]?.trim();
              return (
                <Card key={item.cart_id}>
                  <CardContent className="flex gap-4 p-4">
                    <Link
                      href={`/shop/${item.product_id}`}
                      className="size-20 shrink-0 overflow-hidden rounded-md bg-muted"
                    >
                      {img ? (
                        <img
                          src={img}
                          alt={item.name}
                          className="h-full w-full object-cover"
                        />
                      ) : (
                        <div className="flex h-full w-full items-center justify-center">
                          <Package className="h-6 w-6 text-muted-foreground/50" />
                        </div>
                      )}
                    </Link>
                    <div className="flex flex-1 flex-col justify-between">
                      <div>
                        <Link
                          href={`/shop/${item.product_id}`}
                          className="font-medium hover:text-primary"
                        >
                          {item.name}
                        </Link>
                        {item.sku && (
                          <p className="text-xs text-muted-foreground">
                            SKU: {item.sku}
                          </p>
                        )}
                      </div>
                      <div className="flex items-center justify-between">
                        <div className="flex items-center rounded-lg border">
                          <Button
                            variant="ghost"
                            size="icon-sm"
                            onClick={() =>
                              handleUpdateQuantity(
                                item.cart_id,
                                Math.max(1, item.quantity - 1),
                              )
                            }
                            disabled={item.quantity <= 1}
                          >
                            <Minus className="h-3 w-3" />
                          </Button>
                          <span className="w-8 text-center text-sm">
                            {item.quantity}
                          </span>
                          <Button
                            variant="ghost"
                            size="icon-sm"
                            onClick={() =>
                              handleUpdateQuantity(
                                item.cart_id,
                                item.quantity + 1,
                              )
                            }
                            disabled={item.quantity >= item.stock}
                          >
                            <Plus className="h-3 w-3" />
                          </Button>
                        </div>
                        <div className="flex items-center gap-3">
                          <span className="font-medium">
                            {formatPrice(item.subtotal)}
                          </span>
                          <Button
                            variant="ghost"
                            size="icon-sm"
                            onClick={() => handleRemove(item.cart_id)}
                          >
                            <Trash2 className="h-3.5 w-3.5 text-destructive" />
                          </Button>
                        </div>
                      </div>
                    </div>
                  </CardContent>
                </Card>
              );
            })}
          </div>

          <div>
            <Card>
              <CardContent className="space-y-4 p-4">
                <h3 className="font-semibold">Order Summary</h3>
                <Separator />
                <div className="flex justify-between text-sm">
                  <span className="text-muted-foreground">
                    Items ({items.reduce((s, i) => s + i.quantity, 0)})
                  </span>
                  <span>{formatPrice(total)}</span>
                </div>
                <Separator />
                <div className="flex justify-between font-semibold">
                  <span>Total</span>
                  <span>{formatPrice(total)}</span>
                </div>
                <Link href="/checkout" className="block">
                  <Button className="w-full" size="lg">
                    Proceed to Checkout
                  </Button>
                </Link>
              </CardContent>
            </Card>
          </div>
        </div>
      )}
    </div>
  );
}
