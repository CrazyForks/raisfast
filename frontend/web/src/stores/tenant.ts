import { create } from "zustand";
import { persist } from "zustand/middleware";

interface TenantState {
  currentTenantId: string | null;
  setTenant: (id: string | null) => void;
  clearTenant: () => void;
}

export const useTenantStore = create<TenantState>()(
  persist(
    (set) => ({
      currentTenantId: null,

      setTenant: (id) => set({ currentTenantId: id }),

      clearTenant: () => set({ currentTenantId: null }),
    }),
    { name: "tenant-storage" },
  ),
);
