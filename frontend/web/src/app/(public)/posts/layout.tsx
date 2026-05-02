import type { Metadata } from "next";

export const metadata: Metadata = {
  title: "Posts",
  description: "Browse all published posts",
  openGraph: {
    title: "Posts",
    description: "Browse all published posts",
  },
};

export default function PostsLayout({
  children,
}: {
  children: React.ReactNode;
}) {
  return children;
}
