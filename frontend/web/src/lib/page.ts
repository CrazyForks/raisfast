import { client } from "./raisfast";
import type { PaginatedData } from "@raisfast/sdk";

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
  list: (p = 1, pageSize = 50) =>
    client.send<PaginatedData<Page>>("/pages", {
      query: { page: String(p), page_size: String(pageSize) },
    }),

  getBySlug: (slug: string) =>
    client.send<Page>(`/pages/${slug}`),

  sitemap: () =>
    client.send<{ slug: string; updated_at: string | null }[]>("/pages/sitemap"),

  adminList: (p = 1, pageSize = 50, status?: string) => {
    const query: Record<string, string> = { page: String(p), page_size: String(pageSize) };
    if (status) query.status = status;
    return client.send<PaginatedData<Page>>("/admin/pages", { query });
  },

  adminGet: (id: string) =>
    client.send<Page>(`/admin/pages/${id}`),

  create: (data: Partial<Page>) =>
    client.send<Page>("/pages", { method: "POST", body: data }),

  update: (id: string, data: Partial<Page>) =>
    client.send<Page>(`/admin/pages/${id}`, { method: "PUT", body: data }),

  delete: (id: string) =>
    client.send<void>(`/admin/pages/${id}`, { method: "DELETE" }),

  updateStatus: (id: string, status: string) =>
    client.send<Page>(`/admin/pages/${id}/status`, { method: "PUT", body: { status } }),

  reorder: (items: { id: string; sort_order: number }[]) =>
    client.send<void>("/admin/pages/reorder", { method: "PUT", body: { items } }),

  listReusable: () =>
    client.send<ReusableBlock[]>("/admin/reusable-blocks"),

  getReusable: (id: string) =>
    client.send<ReusableBlock>(`/admin/reusable-blocks/${id}`),

  createReusable: (data: { name: string; block_type: string; content: string; description?: string }) =>
    client.send<ReusableBlock>("/admin/reusable-blocks", { method: "POST", body: data }),

  updateReusable: (id: string, data: Partial<ReusableBlock>) =>
    client.send<ReusableBlock>(`/admin/reusable-blocks/${id}`, { method: "PUT", body: data }),

  deleteReusable: (id: string) =>
    client.send<void>(`/admin/reusable-blocks/${id}`, { method: "DELETE" }),
};
