import { describe, it, expect } from "vitest";
import { render, screen } from "@testing-library/react";
import { PostCard } from "@/components/blog/post-card";
import type { Post } from "@/lib/api";

const basePost: Post = {
  id: "1",
  title: "Test Post",
  slug: "test-post",
  content: "Hello world",
  excerpt: "A test post",
  cover_image: "",
  status: "published",
  created_by: "u1",
  updated_by: null,
  author_name: "Alice",
  category_id: "c1",
  category_name: "Tech",
  tags: [{ id: "t1", name: "rust", slug: "rust" }],
  view_count: 42,
  is_pinned: false,
  created_at: "2025-01-01T00:00:00Z",
  updated_at: "2025-01-01T00:00:00Z",
  published_at: "2025-01-01T00:00:00Z",
  title_highlight: null,
  excerpt_highlight: null,
};

describe("PostCard", () => {
  it("renders title and author", () => {
    render(<PostCard post={basePost} />);
    expect(screen.getByText("Test Post")).toBeInTheDocument();
    expect(screen.getByText("Alice")).toBeInTheDocument();
  });

  it("renders tags", () => {
    render(<PostCard post={basePost} />);
    expect(screen.getByText("rust")).toBeInTheDocument();
  });

  it("renders category badge", () => {
    render(<PostCard post={basePost} />);
    expect(screen.getByText("Tech")).toBeInTheDocument();
  });

  it("renders pinned badge when pinned", () => {
    const pinned = { ...basePost, is_pinned: true };
    render(<PostCard post={pinned} />);
    expect(screen.getByText("Pinned")).toBeInTheDocument();
  });

  it("does not render cover image when empty", () => {
    render(<PostCard post={basePost} />);
    expect(screen.queryByRole("img")).not.toBeInTheDocument();
  });
});
