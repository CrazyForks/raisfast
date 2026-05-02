"use client";

import { useEffect, useState } from "react";
import Link from "next/link";
import { ShoppingCart, Search, Package } from "lucide-react";
import { Card, CardContent } from "@/components/ui/card";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { Input } from "@/components/ui/input";
import { useCartStore } from "@/stores/cart";
import { useAuthStore } from "@/stores/auth";
import { shop, type Product } from "@/lib/ecommerce";
import { toast } from "sonner";
import { useRouter } from "next/navigation";

function formatPrice(price: number) {
  return new Intl.NumberFormat("en-US", {
    style: "currency",
    currency: "USD",
  }).format(price);
}

function ProductCard({ product }: { product: Product }) {
  const addItem = useCartStore((s) => s.addItem);
  const isLoggedIn = useAuthStore((s) => s.isLoggedIn());
  const router = useRouter();
  const images = product.images ? product.images.split(",").filter(Boolean) : [];
  const mainImage = images[0]?.trim();

  async function handleAddToCart(e: React.MouseEvent) {
    e.preventDefault();
    e.stopPropagation();
    if (!isLoggedIn) {
      router.push("/auth/login");
      return;
    }
    try {
      await addItem(product.id);
      toast.success("Added to cart");
    } catch (err) {
      toast.error(err instanceof Error ? err.message : "Failed to add");
    }
  }

  return (
    <Card className="group overflow-hidden transition-shadow hover:shadow-lg">
      <Link href={`/shop/${product.id}`}>
        <div className="aspect-square overflow-hidden bg-muted">
          {mainImage ? (
            <img
              src={mainImage}
              alt={product.name}
              className="h-full w-full object-cover transition-transform duration-300 group-hover:scale-105"
            />
          ) : (
            <div className="flex h-full w-full items-center justify-center">
              <Package className="h-12 w-12 text-muted-foreground/50" />
            </div>
          )}
        </div>
      </Link>
      <CardContent className="space-y-2 p-4">
        <Link href={`/shop/${product.id}`}>
          <h3 className="font-semibold leading-snug line-clamp-2 group-hover:text-primary">
            {product.name}
          </h3>
        </Link>
        <div className="flex items-center gap-2">
          <span className="text-lg font-bold">{formatPrice(product.price)}</span>
          {product.compare_at_price && product.compare_at_price > product.price && (
            <span className="text-sm text-muted-foreground line-through">
              {formatPrice(product.compare_at_price)}
            </span>
          )}
        </div>
        <div className="flex items-center justify-between">
          {product.stock > 0 ? (
            <Badge variant="secondary" className="text-xs">
              {product.stock} in stock
            </Badge>
          ) : (
            <Badge variant="destructive" className="text-xs">Out of stock</Badge>
          )}
          {product.featured && (
            <Badge variant="default" className="text-xs">Featured</Badge>
          )}
        </div>
        <Button
          size="sm"
          className="w-full"
          disabled={product.stock === 0}
          onClick={handleAddToCart}
        >
          <ShoppingCart className="h-3.5 w-3.5" />
          Add to Cart
        </Button>
      </CardContent>
    </Card>
  );
}

export default function ShopPage() {
  const [products, setProducts] = useState<Product[]>([]);
  const [loading, setLoading] = useState(true);
  const [search, setSearch] = useState("");
  const fetchCart = useCartStore((s) => s.fetchCart);
  const isLoggedIn = useAuthStore((s) => s.isLoggedIn());

  useEffect(() => {
    fetchCart();
  }, [fetchCart]);

  useEffect(() => {
    shop
      .listProducts()
      .then((data) => setProducts(data.items))
      .catch(() => toast.error("Failed to load products"))
      .finally(() => setLoading(false));
  }, []);

  const filtered = search
    ? products.filter(
        (p) =>
          p.name.toLowerCase().includes(search.toLowerCase()) ||
          p.description?.toLowerCase().includes(search.toLowerCase()),
      )
    : products;

  return (
    <div className="space-y-6">
      <div className="flex flex-col gap-4 sm:flex-row sm:items-center sm:justify-between">
        <div>
          <h1 className="text-3xl font-bold">Shop</h1>
          <p className="text-muted-foreground">
            Browse our products
          </p>
        </div>
        <div className="relative w-full max-w-xs">
          <Search className="absolute left-2.5 top-1/2 h-4 w-4 -translate-y-1/2 text-muted-foreground" />
          <Input
            placeholder="Search products..."
            value={search}
            onChange={(e) => setSearch(e.target.value)}
            className="pl-8"
          />
        </div>
      </div>

      {loading ? (
        <div className="grid grid-cols-1 gap-6 sm:grid-cols-2 lg:grid-cols-3">
          {Array.from({ length: 6 }).map((_, i) => (
            <Card key={i}>
              <div className="aspect-square animate-pulse bg-muted" />
              <CardContent className="space-y-3 p-4">
                <div className="h-5 w-3/4 animate-pulse rounded bg-muted" />
                <div className="h-6 w-1/3 animate-pulse rounded bg-muted" />
                <div className="h-8 w-full animate-pulse rounded bg-muted" />
              </CardContent>
            </Card>
          ))}
        </div>
      ) : filtered.length === 0 ? (
        <div className="flex flex-col items-center justify-center py-16 text-muted-foreground">
          <Package className="h-12 w-12 mb-4" />
          <p className="text-lg font-medium">No products found</p>
          <p className="text-sm">Check back later for new arrivals</p>
        </div>
      ) : (
        <div className="grid grid-cols-1 gap-6 sm:grid-cols-2 lg:grid-cols-3">
          {filtered.map((product) => (
            <ProductCard key={product.id} product={product} />
          ))}
        </div>
      )}
    </div>
  );
}
