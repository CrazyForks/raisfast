-- 025: 统一系统列命名 + 补齐缺失列
-- author_id → created_by (posts/pages/comments)
-- 新增 updated_by (all 6 built-in tables)
-- 补齐 updated_at (comments/categories/tags)

-- posts
ALTER TABLE posts RENAME COLUMN author_id TO created_by;
ALTER TABLE posts ADD COLUMN updated_by TEXT;

-- pages
ALTER TABLE pages RENAME COLUMN author_id TO created_by;
ALTER TABLE pages ADD COLUMN updated_by TEXT;

-- comments
ALTER TABLE comments RENAME COLUMN author_id TO created_by;
ALTER TABLE comments ADD COLUMN updated_by TEXT;
ALTER TABLE comments ADD COLUMN updated_at TEXT NOT NULL DEFAULT (datetime('now'));

-- categories
ALTER TABLE categories ADD COLUMN created_by TEXT;
ALTER TABLE categories ADD COLUMN updated_by TEXT;
ALTER TABLE categories ADD COLUMN updated_at TEXT NOT NULL DEFAULT (datetime('now'));

-- tags
ALTER TABLE tags ADD COLUMN created_by TEXT;
ALTER TABLE tags ADD COLUMN updated_by TEXT;
ALTER TABLE tags ADD COLUMN updated_at TEXT NOT NULL DEFAULT (datetime('now'));

-- reusable_blocks
ALTER TABLE reusable_blocks ADD COLUMN created_by TEXT;
ALTER TABLE reusable_blocks ADD COLUMN updated_by TEXT;
