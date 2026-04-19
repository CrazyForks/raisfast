"use client";

import { useState } from "react";
import { useRouter } from "next/navigation";
import Link from "next/link";
import { ArrowLeft, Loader2, Package } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Textarea } from "@/components/ui/textarea";
import { Separator } from "@/components/ui/separator";
import { useCartStore } from "@/stores/cart";
import { useAuthStore } from "@/stores/auth";
import { shop, type CheckoutResult } from "@/lib/ecommerce";
import { toast } from "sonner";

function formatPrice(price: number) {
  return new Intl.NumberFormat("en-US", {
    style: "currency",
    currency: "USD",
  }).format(price);
}

export default function CheckoutPage() {
  const router = useRouter();
  const items = useCartStore((s) => s.items);
  const total = useCartStore((s) => s.total);
  const isLoggedIn = useAuthStore((s) => s.isLoggedIn());
  const [shippingAddress, setShippingAddress] = useState("");
  const [note, setNote] = useState("");
  const [submitting, setSubmitting] = useState(false);
  const [result, setResult] = useState<CheckoutResult | null>(null);

  if (!isLoggedIn) {
    router.push("/auth/login");
    return null;
  }

  if (items.length === 0 && !result) {
    router.push("/cart");
    return null;
  }

  async function handleSubmit(e: React.FormEvent) {
    e.preventDefault();
    const userId = useAuthStore.getState().user?.id;
    if (!userId) return;
    setSubmitting(true);
    try {
      const res = await shop.checkout(userId, shippingAddress || undefined, note || undefined);
      setResult(res);
      toast.success("Order placed successfully!");
    } catch (err) {
      toast.error(err instanceof Error ? err.message : "Checkout failed");
    } finally {
      setSubmitting(false);
    }
  }

  if (result) {
    return (
      <div className="flex flex-col items-center justify-center py-16 space-y-4">
        <div className="flex h-16 w-16 items-center justify-center rounded-full bg-green-100 text-green-600">
          <Package className="h-8 w-8" />
        </div>
        <h1 className="text-2xl font-bold">Order Placed!</h1>
        <p className="text-muted-foreground">
          Order <span className="font-mono font-medium">{result.order_no}</span>
        </p>
        <div className="text-center text-sm text-muted-foreground">
          <p>Total: {formatPrice(result.total_amount)}</p>
          <p>{result.items_count} item(s)</p>
        </div>
        <div className="flex gap-3">
          <Link href="/orders">
            <Button>View Orders</Button>
          </Link>
          <Link href="/shop">
            <Button variant="outline">Continue Shopping</Button>
          </Link>
        </div>
      </div>
    );
  }

  return (
    <div className="space-y-6">
      <div className="flex items-center gap-3">
        <Link
          href="/cart"
          className="inline-flex items-center gap-1 text-sm text-muted-foreground hover:text-foreground"
        >
          <ArrowLeft className="h-4 w-4" />
          Back to Cart
        </Link>
      </div>
      <h1 className="text-2xl font-bold">Checkout</h1>

      <div className="grid gap-6 lg:grid-cols-3">
        <div className="lg:col-span-2">
          <Card>
            <CardContent className="space-y-4 p-4">
              <h3 className="font-semibold">Shipping Information</h3>
              <Separator />
              <form onSubmit={handleSubmit} className="space-y-4">
                <div className="space-y-2">
                  <Label htmlFor="address">Shipping Address</Label>
                  <Input
                    id="address"
                    placeholder="Enter your shipping address"
                    value={shippingAddress}
                    onChange={(e) => setShippingAddress(e.target.value)}
                  />
                </div>
                <div className="space-y-2">
                  <Label htmlFor="note">Order Note (optional)</Label>
                  <Textarea
                    id="note"
                    placeholder="Any special instructions..."
                    value={note}
                    onChange={(e) => setNote(e.target.value)}
                    rows={3}
                  />
                </div>
                <Button type="submit" size="lg" className="w-full" disabled={submitting}>
                  {submitting && <Loader2 className="h-4 w-4 animate-spin" />}
                  Place Order — {formatPrice(total)}
                </Button>
              </form>
            </CardContent>
          </Card>
        </div>

        <div>
          <Card>
            <CardContent className="space-y-3 p-4">
              <h3 className="font-semibold">Order Items</h3>
              <Separator />
              {items.map((item) => (
                <div key={item.cart_id} className="flex justify-between text-sm">
                  <span className="text-muted-foreground">
                    {item.name} x{item.quantity}
                  </span>
                  <span>{formatPrice(item.subtotal)}</span>
                </div>
              ))}
              <Separator />
              <div className="flex justify-between font-semibold">
                <span>Total</span>
                <span>{formatPrice(total)}</span>
              </div>
            </CardContent>
          </Card>
        </div>
      </div>
    </div>
  );
}
