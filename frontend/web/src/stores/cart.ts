"use client";

import { create } from "zustand";
import { shop, type Cart, type CartItem } from "@/lib/ecommerce";
import { useAuthStore } from "./auth";

interface CartState {
  items: CartItem[];
  total: number;
  loading: boolean;
  fetchCart: () => Promise<void>;
  addItem: (productId: string, quantity?: number) => Promise<void>;
  updateItem: (itemId: string, quantity: number) => Promise<void>;
  removeItem: (itemId: string) => Promise<void>;
  clearAll: () => Promise<void>;
  itemCount: () => number;
}

export const useCartStore = create<CartState>()((set, get) => ({
  items: [],
  total: 0,
  loading: false,

  fetchCart: async () => {
    const userId = useAuthStore.getState().user?.id;
    if (!userId) return;
    set({ loading: true });
    try {
      const cart = await shop.viewCart(userId);
      set({ items: cart.items, total: cart.total, loading: false });
    } catch {
      set({ items: [], total: 0, loading: false });
    }
  },

  addItem: async (productId, quantity = 1) => {
    const userId = useAuthStore.getState().user?.id;
    if (!userId) return;
    await shop.addToCart(userId, productId, quantity);
    await get().fetchCart();
  },

  updateItem: async (itemId, quantity) => {
    await shop.updateCartItem(itemId, quantity);
    await get().fetchCart();
  },

  removeItem: async (itemId) => {
    await shop.removeCartItem(itemId);
    await get().fetchCart();
  },

  clearAll: async () => {
    const userId = useAuthStore.getState().user?.id;
    if (!userId) return;
    await shop.clearCart(userId);
    set({ items: [], total: 0 });
  },

  itemCount: () => {
    return get().items.reduce((sum, item) => sum + item.quantity, 0);
  },
}));
