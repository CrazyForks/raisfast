"use client";

import { useEffect, useState } from "react";
import { useParams, useRouter } from "next/navigation";
import Link from "next/link";
import { ArrowLeft, Clock, CheckCircle, XCircle, Truck, Package } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { Card, CardContent } from "@/components/ui/card";
import { Separator } from "@/components/ui/separator";
import { useAuthStore } from "@/stores/auth";
import { shop, type Order } from "@/lib/ecommerce";
import { toast } from "sonner";

function formatPrice(price: number) {
  return new Intl.NumberFormat("en-US", {
    style: "currency",
    currency: "USD",
  }).format(price);
}

function formatDate(dateStr: string) {
  return new Date(dateStr).toLocaleDateString("en-US", {
    year: "numeric",
    month: "short",
    day: "numeric",
    hour: "2-digit",
    minute: "2-digit",
  });
}

const statusConfig: Record<string, { icon: React.ElementType; variant: "default" | "secondary" | "destructive" | "outline" }> = {
  pending: { icon: Clock, variant: "secondary" },
  paid: { icon: CheckCircle, variant: "default" },
  shipped: { icon: Truck, variant: "default" },
  delivered: { icon: CheckCircle, variant: "default" },
  cancelled: { icon: XCircle, variant: "destructive" },
};

function StatusBadge({ status }: { status: string }) {
  const config = statusConfig[status] || statusConfig.pending;
  const Icon = config.icon;
  return (
    <Badge variant={config.variant} className="gap-1 capitalize">
      <Icon className="h-3 w-3" />
      {status}
    </Badge>
  );
}

export default function OrderDetailPage() {
  const params = useParams();
  const router = useRouter();
  const orderId = params.id as string;
  const isLoggedIn = useAuthStore((s) => s.isLoggedIn());
  const [order, setOrder] = useState<Order | null>(null);
  const [loading, setLoading] = useState(true);

  useEffect(() => {
    if (!isLoggedIn) {
      router.push("/auth/login");
      return;
    }
    const userId = useAuthStore.getState().user?.id;
    if (!userId) return;
    shop
      .getOrder(orderId, userId)
      .then(setOrder)
      .catch(() => toast.error("Order not found"))
      .finally(() => setLoading(false));
  }, [orderId, isLoggedIn, router]);

  if (!isLoggedIn) return null;

  if (loading) {
    return (
      <div className="space-y-6">
        <div className="h-8 w-24 animate-pulse rounded bg-muted" />
        <div className="space-y-4">
          <div className="h-6 w-48 animate-pulse rounded bg-muted" />
          <div className="h-40 w-full animate-pulse rounded bg-muted" />
        </div>
      </div>
    );
  }

  if (!order) {
    return (
      <div className="flex flex-col items-center justify-center py-16 text-muted-foreground">
        <Package className="h-12 w-12 mb-4" />
        <p className="text-lg font-medium">Order not found</p>
        <Link href="/orders">
          <Button variant="outline" className="mt-4">
            <ArrowLeft className="h-4 w-4" />
            Back to Orders
          </Button>
        </Link>
      </div>
    );
  }

  return (
    <div className="space-y-6">
      <Link
        href="/orders"
        className="inline-flex items-center gap-1 text-sm text-muted-foreground hover:text-foreground"
      >
        <ArrowLeft className="h-4 w-4" />
        Back to Orders
      </Link>

      <div className="flex items-center justify-between">
        <div>
          <h1 className="text-2xl font-bold">Order {order.order_no}</h1>
          <p className="text-sm text-muted-foreground">
            {formatDate(order.created_at)}
          </p>
        </div>
        <StatusBadge status={order.status} />
      </div>

      <div className="grid gap-6 lg:grid-cols-3">
        <div className="lg:col-span-2">
          <Card>
            <CardContent className="p-4">
              <h3 className="font-semibold mb-3">Items</h3>
              <div className="space-y-3">
                {order.items?.map((item) => (
                  <div key={item.id}>
                    <div className="flex items-center justify-between text-sm">
                      <div>
                        <span className="font-medium">{item.product_name}</span>
                        <span className="text-muted-foreground ml-2">
                          x{item.quantity}
                        </span>
                      </div>
                      <span>{formatPrice(item.subtotal)}</span>
                    </div>
                    <div className="text-xs text-muted-foreground">
                      {formatPrice(item.price)} each
                    </div>
                    <Separator className="mt-3" />
                  </div>
                ))}
              </div>
            </CardContent>
          </Card>
        </div>

        <div className="space-y-4">
          <Card>
            <CardContent className="space-y-3 p-4">
              <h3 className="font-semibold">Summary</h3>
              <Separator />
              <div className="flex justify-between text-sm">
                <span className="text-muted-foreground">
                  Items ({order.items?.reduce((s, i) => s + i.quantity, 0) ?? 0})
                </span>
                <span>{formatPrice(order.total_amount)}</span>
              </div>
              <Separator />
              <div className="flex justify-between font-semibold">
                <span>Total</span>
                <span>{formatPrice(order.total_amount)}</span>
              </div>
            </CardContent>
          </Card>

          {(order.shipping_address || order.note) && (
            <Card>
              <CardContent className="space-y-3 p-4">
                {order.shipping_address && (
                  <div>
                    <h4 className="text-sm font-medium mb-1">Shipping Address</h4>
                    <p className="text-sm text-muted-foreground">
                      {order.shipping_address}
                    </p>
                  </div>
                )}
                {order.note && (
                  <div>
                    <h4 className="text-sm font-medium mb-1">Note</h4>
                    <p className="text-sm text-muted-foreground">{order.note}</p>
                  </div>
                )}
              </CardContent>
            </Card>
          )}

          <Card>
            <CardContent className="space-y-2 p-4">
              <h3 className="font-semibold mb-2">Timeline</h3>
              <div className="space-y-1 text-sm">
                <div className="flex justify-between">
                  <span className="text-muted-foreground">Created</span>
                  <span>{formatDate(order.created_at)}</span>
                </div>
                {order.paid_at && (
                  <div className="flex justify-between">
                    <span className="text-muted-foreground">Paid</span>
                    <span>{formatDate(order.paid_at)}</span>
                  </div>
                )}
                {order.shipped_at && (
                  <div className="flex justify-between">
                    <span className="text-muted-foreground">Shipped</span>
                    <span>{formatDate(order.shipped_at)}</span>
                  </div>
                )}
              </div>
            </CardContent>
          </Card>
        </div>
      </div>
    </div>
  );
}
