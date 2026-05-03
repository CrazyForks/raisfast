import type { MetadataRoute } from "next";

const BASE_URL = process.env.NEXT_PUBLIC_SITE_URL || "http://localhost:3000";
const API_URL = process.env.NEXT_PUBLIC_API_URL || "http://localhost:9898/api/v1";

export default async function sitemap(): Promise<MetadataRoute.Sitemap> {
  const staticPages: MetadataRoute.Sitemap = [
    { url: BASE_URL, lastModified: new Date(), changeFrequency: "daily", priority: 1 },
    { url: `${BASE_URL}/posts`, lastModified: new Date(), changeFrequency: "daily", priority: 0.9 },
  ];

  try {
    const res = await fetch(`${API_URL}/posts?status=published&page_size=100`, {
      next: { revalidate: 3600 },
    });
    const data = await res.json();
    const posts: Array<{ slug: string; updated_at?: string }> = data?.data?.items ?? [];

    const postPages: MetadataRoute.Sitemap = posts.map((post) => ({
      url: `${BASE_URL}/posts/${post.slug}`,
      lastModified: post.updated_at ? new Date(post.updated_at) : new Date(),
      changeFrequency: "weekly" as const,
      priority: 0.8,
    }));

    return [...staticPages, ...postPages];
  } catch {
    return staticPages;
  }
}
