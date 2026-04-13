CREATE INDEX IF NOT EXISTS idx_posts_status_created
    ON posts(status, is_pinned DESC, created_at DESC);

CREATE INDEX IF NOT EXISTS idx_posts_status_category
    ON posts(status, category_id);

CREATE INDEX IF NOT EXISTS idx_posts_status_author
    ON posts(status, author_id);

CREATE INDEX IF NOT EXISTS idx_comments_post_status
    ON comments(post_id, status);

CREATE INDEX IF NOT EXISTS idx_comments_parent_id
    ON comments(parent_id);

CREATE INDEX IF NOT EXISTS idx_posts_tags_tag_id
    ON posts_tags(tag_id);

CREATE INDEX IF NOT EXISTS idx_refresh_tokens_expires_at
    ON refresh_tokens(expires_at);

CREATE INDEX IF NOT EXISTS idx_media_user_created
    ON media(user_id, created_at DESC);
