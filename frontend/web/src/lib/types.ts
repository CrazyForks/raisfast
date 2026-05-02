export interface Post {
  id: string;
  title: string;
  slug: string;
  content: string;
  excerpt: string;
  cover_image: string;
  status: string;
  created_by: string;
  updated_by: string | null;
  author_name: string;
  category_id: string;
  category_name: string;
  tags: { id: string; name: string; slug: string }[];
  view_count: number;
  is_pinned: boolean;
  created_at: string;
  updated_at: string;
  published_at: string;
  title_highlight: string | null;
  excerpt_highlight: string | null;
}

export interface Comment {
  id: string;
  content: string;
  author_name: string;
  nickname: string;
  created_at: string;
  replies?: Comment[];
}
