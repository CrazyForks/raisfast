import { create } from "zustand";
import { persist } from "zustand/middleware";

interface TenantState {
  currentTenantId: string | null;
  builtinTenantable: boolean;
  setTenant: (id: string | null) => void;
  clearTenant: () => void;
  setBuiltinTenantable: (v: boolean) => void;
}

export const useTenantStore = create<TenantState>()(
  persist(
    (set) => ({
      currentTenantId: null,
      builtinTenantable: false,

      setTenant: (id) => set({ currentTenantId: id }),

      clearTenant: () => set({ currentTenantId: null }),

      setBuiltinTenantable: (v) => set({ builtinTenantable: v }),
    }),
    { name: "tenant-storage", partialize: (state) => ({ currentTenantId: state.currentTenantId }) },
  ),
);
