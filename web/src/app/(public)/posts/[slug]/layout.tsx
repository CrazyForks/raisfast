import type { Metadata } from "next";

const API_URL = process.env.NEXT_PUBLIC_API_URL || "http://localhost:9000/api/v1";
const SITE_URL = process.env.NEXT_PUBLIC_SITE_URL || "http://localhost:3000";

type PostData = {
  title?: string;
  excerpt?: string;
  slug?: string;
  html_content?: string;
  author_name?: string;
  published_at?: string;
};

export async function generateMetadata({
  params,
}: {
  params: Promise<{ slug: string }>;
}): Promise<Metadata> {
  const { slug } = await params;

  try {
    const res = await fetch(`${API_URL}/posts/${slug}`, { next: { revalidate: 300 } });
    if (!res.ok) return { title: "Post not found" };
    const body = await res.json();
    const post: PostData = body?.data ?? body;
    const description = post.excerpt || `Read "${post.title}" on Blog`;

    return {
      title: post.title,
      description,
      openGraph: {
        title: post.title,
        description,
        type: "article",
        url: `${SITE_URL}/posts/${slug}`,
        publishedTime: post.published_at,
        authors: post.author_name ? [post.author_name] : undefined,
      },
      twitter: {
        card: "summary_large_image",
        title: post.title,
        description,
      },
    };
  } catch {
    return { title: "Post" };
  }
}

export default function PostSlugLayout({
  children,
}: {
  children: React.ReactNode;
}) {
  return children;
}
