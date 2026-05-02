"use client";

import { useEffect, useState } from "react";
import { useParams, useRouter } from "next/navigation";
import Link from "next/link";
import {
  ArrowLeft,
  Minus,
  Plus,
  ShoppingCart,
  Package,
  Star,
} from "lucide-react";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { Card, CardContent } from "@/components/ui/card";
import { Separator } from "@/components/ui/separator";
import { useCartStore } from "@/stores/cart";
import { useAuthStore } from "@/stores/auth";
import { shop, type Product } from "@/lib/ecommerce";
import { toast } from "sonner";

function formatPrice(price: number) {
  return new Intl.NumberFormat("en-US", {
    style: "currency",
    currency: "USD",
  }).format(price);
}

export default function ProductDetailPage() {
  const params = useParams();
  const router = useRouter();
  const productId = params.id as string;
  const [product, setProduct] = useState<Product | null>(null);
  const [loading, setLoading] = useState(true);
  const [quantity, setQuantity] = useState(1);
  const [selectedImage, setSelectedImage] = useState(0);
  const addItem = useCartStore((s) => s.addItem);
  const isLoggedIn = useAuthStore((s) => s.isLoggedIn());

  useEffect(() => {
    shop
      .getProduct(productId)
      .then(setProduct)
      .catch(() => toast.error("Product not found"))
      .finally(() => setLoading(false));
  }, [productId]);

  if (loading) {
    return (
      <div className="space-y-6">
        <div className="h-8 w-24 animate-pulse rounded bg-muted" />
        <div className="grid gap-8 md:grid-cols-2">
          <div className="aspect-square animate-pulse rounded-lg bg-muted" />
          <div className="space-y-4">
            <div className="h-8 w-3/4 animate-pulse rounded bg-muted" />
            <div className="h-6 w-1/4 animate-pulse rounded bg-muted" />
            <div className="h-20 w-full animate-pulse rounded bg-muted" />
          </div>
        </div>
      </div>
    );
  }

  if (!product) {
    return (
      <div className="flex flex-col items-center justify-center py-16 text-muted-foreground">
        <Package className="h-12 w-12 mb-4" />
        <p className="text-lg font-medium">Product not found</p>
        <Link href="/shop">
          <Button variant="outline" className="mt-4">
            <ArrowLeft className="h-4 w-4" />
            Back to Shop
          </Button>
        </Link>
      </div>
    );
  }

  const images = product.images ? product.images.split(",").filter(Boolean).map((s) => s.trim()) : [];

  async function handleAddToCart() {
    if (!product) return;
    if (!isLoggedIn) {
      router.push("/auth/login");
      return;
    }
    try {
      await addItem(product.id, quantity);
      toast.success("Added to cart");
    } catch (err) {
      toast.error(err instanceof Error ? err.message : "Failed to add");
    }
  }

  const discount =
    product.compare_at_price && product.compare_at_price > product.price
      ? Math.round(
          ((product.compare_at_price - product.price) /
            product.compare_at_price) *
            100,
        )
      : 0;

  return (
    <div className="space-y-6">
      <Link
        href="/shop"
        className="inline-flex items-center gap-1 text-sm text-muted-foreground hover:text-foreground"
      >
        <ArrowLeft className="h-4 w-4" />
        Back to Shop
      </Link>

      <div className="grid gap-8 md:grid-cols-2">
        <div className="space-y-3">
          <div className="aspect-square overflow-hidden rounded-lg bg-muted">
            {images.length > 0 ? (
              <img
                src={images[selectedImage]}
                alt={product.name}
                className="h-full w-full object-cover"
              />
            ) : (
              <div className="flex h-full w-full items-center justify-center">
                <Package className="h-16 w-16 text-muted-foreground/50" />
              </div>
            )}
          </div>
          {images.length > 1 && (
            <div className="flex gap-2 overflow-x-auto">
              {images.map((img, i) => (
                <button
                  key={i}
                  onClick={() => setSelectedImage(i)}
                  className={`size-16 shrink-0 overflow-hidden rounded-md border-2 ${
                    i === selectedImage
                      ? "border-primary"
                      : "border-transparent"
                  }`}
                >
                  <img
                    src={img}
                    alt={`${product.name} ${i + 1}`}
                    className="h-full w-full object-cover"
                  />
                </button>
              ))}
            </div>
          )}
        </div>

        <div className="space-y-6">
          <div>
            <div className="flex items-center gap-2 mb-2">
              {product.featured && (
                <Badge variant="default">
                  <Star className="h-3 w-3" />
                  Featured
                </Badge>
              )}
              {product.sku && (
                <Badge variant="outline" className="text-xs">
                  SKU: {product.sku}
                </Badge>
              )}
            </div>
            <h1 className="text-2xl font-bold md:text-3xl">{product.name}</h1>
          </div>

          <div className="flex items-baseline gap-3">
            <span className="text-3xl font-bold">
              {formatPrice(product.price)}
            </span>
            {product.compare_at_price &&
              product.compare_at_price > product.price && (
                <>
                  <span className="text-lg text-muted-foreground line-through">
                    {formatPrice(product.compare_at_price)}
                  </span>
                  <Badge variant="destructive">-{discount}%</Badge>
                </>
              )}
          </div>

          <Separator />

          {product.description && (
            <div className="text-sm text-muted-foreground whitespace-pre-line">
              {product.description}
            </div>
          )}

          <Separator />

          <div className="space-y-4">
            <div className="flex items-center gap-3 text-sm">
              <span className="text-muted-foreground">Availability:</span>
              {product.stock > 0 ? (
                <Badge variant="secondary">{product.stock} in stock</Badge>
              ) : (
                <Badge variant="destructive">Out of stock</Badge>
              )}
            </div>

            {product.weight && (
              <div className="flex items-center gap-3 text-sm">
                <span className="text-muted-foreground">Weight:</span>
                <span>{product.weight}g</span>
              </div>
            )}

            {product.stock > 0 && (
              <div className="flex items-center gap-3">
                <div className="flex items-center rounded-lg border">
                  <Button
                    variant="ghost"
                    size="icon-sm"
                    onClick={() => setQuantity(Math.max(1, quantity - 1))}
                    disabled={quantity <= 1}
                  >
                    <Minus className="h-3 w-3" />
                  </Button>
                  <span className="w-10 text-center text-sm font-medium">
                    {quantity}
                  </span>
                  <Button
                    variant="ghost"
                    size="icon-sm"
                    onClick={() =>
                      setQuantity(Math.min(product.stock, quantity + 1))
                    }
                    disabled={quantity >= product.stock}
                  >
                    <Plus className="h-3 w-3" />
                  </Button>
                </div>
              </div>
            )}

            <div className="flex gap-3">
              <Button
                size="lg"
                className="flex-1"
                disabled={product.stock === 0}
                onClick={handleAddToCart}
              >
                <ShoppingCart className="h-4 w-4" />
                Add to Cart
              </Button>
              {isLoggedIn && (
                <Link href="/cart">
                  <Button variant="outline" size="lg">
                    View Cart
                  </Button>
                </Link>
              )}
            </div>
          </div>
        </div>
      </div>
    </div>
  );
}
