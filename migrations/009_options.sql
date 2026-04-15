-- 站点配置 KV 表
-- 参考 WordPress wp_options 设计，支持启动时预加载

CREATE TABLE IF NOT EXISTS options (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL,
    autoload BOOLEAN NOT NULL DEFAULT 1,
    updated_at TEXT NOT NULL
);

-- 预置默认配置
INSERT OR IGNORE INTO options (key, value, autoload, updated_at) VALUES
    ('site_title', '"My Blog"', 1, datetime('now')),
    ('site_description', '""', 1, datetime('now')),
    ('posts_per_page', '10', 1, datetime('now')),
    ('default_role', '"reader"', 1, datetime('now')),
    ('comment_moderation', 'true', 1, datetime('now')),
    ('comment_order', '"asc"', 1, datetime('now')),
    ('theme', '"default"', 1, datetime('now')),
    ('timezone', '"UTC"', 1, datetime('now')),
    ('date_format', '"%Y-%m-%d"', 1, datetime('now')),
    ('permalink_structure', '"/:year/:month/:slug"', 1, datetime('now')),
    ('rss_items', '20', 1, datetime('now')),
    ('maintenance_mode', 'false', 1, datetime('now'));
