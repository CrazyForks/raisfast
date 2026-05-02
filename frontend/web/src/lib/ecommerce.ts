import { client } from "@/lib/raisfast";

const BASE = "/plugins/ecommerce";

export interface Product {
  id: string;
  name: string;
  slug: string;
  description?: string;
  price: number;
  compare_at_price?: number | null;
  stock: number;
  sku?: string;
  images?: string | null;
  featured?: boolean;
  weight?: number;
}

export interface CartItem {
  cart_id: string;
  product_id: string;
  quantity: number;
  name: string;
  price: number;
  stock: number;
  images?: string | null;
  sku?: string;
  subtotal: number;
}

export interface Cart {
  items: CartItem[];
  total: number;
}

export interface Order {
  id: string;
  order_no: string;
  user_id: string;
  status: string;
  total_amount: number;
  shipping_address?: string | null;
  note?: string | null;
  paid_at?: string | null;
  shipped_at?: string | null;
  created_at: string;
  items?: OrderItem[];
}

export interface OrderItem {
  id: string;
  product_id: string;
  product_name: string;
  price: number;
  quantity: number;
  subtotal: number;
}

export interface CheckoutResult {
  order_id: string;
  order_no: string;
  status: string;
  total_amount: number;
  items_count: number;
}

interface PluginResponse<T> {
  ok: boolean;
  data?: T;
  error?: string;
}

async function pluginGet<T>(path: string, body?: object): Promise<T> {
  const opts: { method: string; body?: object } = { method: "GET" };
  if (body) opts.body = body;
  const res = await client.send<PluginResponse<T>>(BASE + path, opts);
  if (!res.ok || !res.data) throw new Error(res.error ?? "Unknown error");
  return res.data;
}

async function pluginPost<T>(path: string, body?: object): Promise<T> {
  const res = await client.send<PluginResponse<T>>(BASE + path, { method: "POST", body });
  if (!res.ok || !res.data) throw new Error(res.error ?? "Unknown error");
  return res.data;
}

async function pluginPut<T>(path: string, body?: object): Promise<T> {
  const res = await client.send<PluginResponse<T>>(BASE + path, { method: "PUT", body });
  if (!res.ok || !res.data) throw new Error(res.error ?? "Unknown error");
  return res.data;
}

async function pluginDelete<T>(path: string, body?: object): Promise<T> {
  const opts: { method: string; body?: object } = { method: "DELETE" };
  if (body) opts.body = body;
  const res = await client.send<PluginResponse<T>>(BASE + path, opts);
  if (!res.ok || !res.data) throw new Error(res.error ?? "Unknown error");
  return res.data;
}

export const shop = {
  listProducts: () =>
    pluginGet<{ items: Product[]; total: number }>("/products"),

  getProduct: (id: string) => pluginGet<Product>(`/products/${id}`),

  viewCart: (userId: string) =>
    pluginGet<Cart>("/cart", { user_id: userId }).catch(() => ({
      items: [],
      total: 0,
    })),

  addToCart: (userId: string, productId: string, quantity = 1) =>
    pluginPost<{ added: boolean }>("/cart", {
      user_id: userId,
      product_id: productId,
      quantity,
    }),

  updateCartItem: (itemId: string, quantity: number) =>
    pluginPut<{ updated: boolean }>(`/cart/${itemId}`, { quantity }),

  removeCartItem: (itemId: string) =>
    pluginDelete<{ removed: boolean }>(`/cart/${itemId}`),

  clearCart: (userId: string) =>
    pluginDelete<{ cleared: boolean }>("/cart"),

  checkout: (
    userId: string,
    shippingAddress?: string,
    note?: string,
  ) =>
    pluginPost<CheckoutResult>("/checkout", {
      user_id: userId,
      shipping_address: shippingAddress,
      note,
    }),

  listOrders: (userId: string) =>
    pluginGet<{ items: Order[]; total: number }>("/orders", {
      user_id: userId,
    }),

  getOrder: (orderId: string, userId: string) =>
    pluginGet<Order>(`/orders/${orderId}`, { user_id: userId }),
};
