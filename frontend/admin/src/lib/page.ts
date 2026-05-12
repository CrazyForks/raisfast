import { client } from "./raisfast";
import type { Page as PageType, ReusableBlock, PageStatus, CreatePageBody, UpdatePageBody } from "@raisfast/sdk";

export type Page = Omit<PageType, "id"> & { id: string };
export type { ReusableBlock };

export const page = {
  list: (p = 1, pageSize = 50) =>
    client.pages.list(p, pageSize),

  getBySlug: (slug: string) =>
    client.pages.get(slug),

  sitemap: () =>
    client.pages.sitemap(),

  adminList: async (p = 1, pageSize = 50, status?: string) => {
    const res = await client.admin.pages.list({ page: p, page_size: pageSize, status: status as PageStatus | undefined });
    return { ...res, items: res.items.map((pg) => ({ ...pg, id: String(pg.id) })) };
  },

  adminGet: async (id: string) => {
    const res = await client.admin.pages.get(id);
    return { ...res, id: String(res.id) };
  },

  create: (data: CreatePageBody) =>
    client.admin.pages.create(data),

  update: (id: string, data: UpdatePageBody) =>
    client.admin.pages.update(id, data),

  delete: (id: string) =>
    client.admin.pages.delete(id),

  updateStatus: (id: string, status: string) =>
    client.admin.pages.updateStatus(id, status as PageStatus),

  reorder: (items: { id: string; sort_order: number }[]) =>
    client.admin.pages.reorder(items),

  listReusable: async () => {
    const res = await client.admin.reusableBlocks.list();
    return res.map((rb) => ({ ...rb, id: String(rb.id) }));
  },

  getReusable: async (id: string) => {
    const res = await client.admin.reusableBlocks.get(id);
    return { ...res, id: String(res.id) };
  },

  createReusable: (data: { name: string; block_type: string; content: string; description?: string }) =>
    client.admin.reusableBlocks.create(data),

  updateReusable: (id: string, data: Record<string, unknown>) =>
    client.admin.reusableBlocks.update(id, data as Parameters<typeof client.admin.reusableBlocks.update>[1]),

  deleteReusable: (id: string) =>
    client.admin.reusableBlocks.delete(id),
};
