"use client";

import { useEffect, useState } from "react";
import Link from "next/link";
import { useRouter } from "next/navigation";
import { Package, Clock, CheckCircle, XCircle, Truck } from "lucide-react";
import { Card, CardContent } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
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

export default function OrdersPage() {
  const router = useRouter();
  const isLoggedIn = useAuthStore((s) => s.isLoggedIn());
  const [orders, setOrders] = useState<Order[]>([]);
  const [loading, setLoading] = useState(true);

  useEffect(() => {
    if (!isLoggedIn) {
      router.push("/auth/login");
      return;
    }
    const userId = useAuthStore.getState().user?.id;
    if (!userId) return;
    shop
      .listOrders(userId)
      .then((data) => setOrders(data.items))
      .catch(() => toast.error("Failed to load orders"))
      .finally(() => setLoading(false));
  }, [isLoggedIn, router]);

  if (!isLoggedIn) return null;

  return (
    <div className="space-y-6">
      <div className="flex items-center justify-between">
        <h1 className="text-2xl font-bold">My Orders</h1>
        <Link href="/shop">
          <Button variant="outline" size="sm">Continue Shopping</Button>
        </Link>
      </div>

      {loading ? (
        <div className="space-y-4">
          {Array.from({ length: 3 }).map((_, i) => (
            <Card key={i}>
              <CardContent className="p-4">
                <div className="h-5 w-32 animate-pulse rounded bg-muted" />
                <div className="mt-2 h-4 w-48 animate-pulse rounded bg-muted" />
              </CardContent>
            </Card>
          ))}
        </div>
      ) : orders.length === 0 ? (
        <div className="flex flex-col items-center justify-center py-16 text-muted-foreground">
          <Package className="h-12 w-12 mb-4" />
          <p className="text-lg font-medium">No orders yet</p>
          <p className="text-sm mb-4">Place your first order from our shop</p>
          <Link href="/shop">
            <Button>Browse Products</Button>
          </Link>
        </div>
      ) : (
        <div className="space-y-4">
          {orders.map((order) => (
            <Link key={order.id} href={`/orders/${order.id}`}>
              <Card className="transition-shadow hover:shadow-md cursor-pointer">
                <CardContent className="p-4">
                  <div className="flex items-center justify-between">
                    <div className="space-y-1">
                      <div className="flex items-center gap-2">
                        <span className="font-mono font-medium text-sm">
                          {order.order_no}
                        </span>
                        <StatusBadge status={order.status} />
                      </div>
                      <p className="text-xs text-muted-foreground">
                        {formatDate(order.created_at)}
                      </p>
                    </div>
                    <span className="font-semibold">
                      {formatPrice(order.total_amount)}
                    </span>
                  </div>
                </CardContent>
              </Card>
            </Link>
          ))}
        </div>
      )}
    </div>
  );
}
