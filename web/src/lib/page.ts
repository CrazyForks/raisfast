import { api, type PaginatedData } from "./api";

export interface Page {
  id: string;
  title: string;
  slug: string;
  content: string | null;
  blocks: string | null;
  meta_title: string | null;
  meta_description: string | null;
  og_image: string | null;
  template: string;
  parent_id: string | null;
  sort_order: number;
  status: string;
  created_by: string;
  updated_by: string | null;
  cover_image: string | null;
  published_at: string | null;
  created_at: string;
  updated_at: string;
}

export interface ReusableBlock {
  id: string;
  name: string;
  block_type: string;
  content: string;
  description: string | null;
  created_at: string;
  updated_at: string;
}

export const page = {
  list: (page = 1, pageSize = 50) =>
    api.get<PaginatedData<Page>>(`/pages?page=${page}&page_size=${pageSize}`),

  getBySlug: (slug: string) =>
    api.get<Page>(`/pages/${slug}`),

  sitemap: () =>
    api.get<{ slug: string; updated_at: string | null }[]>("/pages/sitemap"),

  adminList: (page = 1, pageSize = 50, status?: string) => {
    const p = new URLSearchParams({ page: String(page), page_size: String(pageSize) });
    if (status) p.set("status", status);
    return api.get<PaginatedData<Page>>(`/admin/pages?${p.toString()}`);
  },

  adminGet: (id: string) =>
    api.get<Page>(`/admin/pages/${id}`),

  create: (data: Partial<Page>) =>
    api.post<Page>("/pages", data),

  update: (id: string, data: Partial<Page>) =>
    api.put<Page>(`/admin/pages/${id}`, data),

  delete: (id: string) =>
    api.delete(`/admin/pages/${id}`),

  updateStatus: (id: string, status: string) =>
    api.put<Page>(`/admin/pages/${id}/status`, { status }),

  reorder: (items: { id: string; sort_order: number }[]) =>
    api.put("/admin/pages/reorder", { items }),

  listReusable: () =>
    api.get<ReusableBlock[]>("/admin/reusable-blocks"),

  getReusable: (id: string) =>
    api.get<ReusableBlock>(`/admin/reusable-blocks/${id}`),

  createReusable: (data: { name: string; block_type: string; content: string; description?: string }) =>
    api.post<ReusableBlock>("/admin/reusable-blocks", data),

  updateReusable: (id: string, data: Partial<ReusableBlock>) =>
    api.put<ReusableBlock>(`/admin/reusable-blocks/${id}`, data),

  deleteReusable: (id: string) =>
    api.delete(`/admin/reusable-blocks/${id}`),
};
