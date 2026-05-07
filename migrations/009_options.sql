-- 站点配置表 v2
-- 增加类型、分组、元数据、租户支持

DROP TABLE IF EXISTS options;

CREATE TABLE options (
    id TEXT PRIMARY KEY,
    key TEXT NOT NULL,
    value TEXT NOT NULL,
    type TEXT NOT NULL DEFAULT 'text',
    group_name TEXT NOT NULL DEFAULT 'general',
    label TEXT NOT NULL DEFAULT '',
    description TEXT,
    validation TEXT,
    is_public BOOLEAN NOT NULL DEFAULT 0,
    autoload BOOLEAN NOT NULL DEFAULT 1,
    sort_order INTEGER NOT NULL DEFAULT 0,
    updated_at TEXT NOT NULL,
    UNIQUE(key)
);

-- ── 常规 ──────────────────────────────────────────────
INSERT INTO options (id, key, value, type, group_name, label, description, validation, is_public, autoload, sort_order, updated_at) VALUES
    ('opt-site-title', 'site_title', '"My Blog"', 'text', 'general', '站点标题', '显示在浏览器标题栏和页面头部', '{"max_length":100}', 1, 1, 1, datetime('now')),
    ('opt-site-desc', 'site_description', '""', 'text', 'general', '站点描述', '简短描述站点用途', '{"max_length":500}', 1, 1, 2, datetime('now')),
    ('opt-site-url', 'site_url', '""', 'url', 'general', '站点 URL', '如 https://example.com', NULL, 1, 1, 3, datetime('now')),
    ('opt-admin-email', 'admin_email', '""', 'email', 'general', '管理员邮箱', NULL, NULL, 0, 1, 4, datetime('now')),
    ('opt-timezone', 'timezone', '"UTC"', 'select', 'general', '时区', NULL, '{"values":["UTC","Asia/Shanghai","Asia/Tokyo","US/Eastern","US/Pacific","Europe/London","Europe/Berlin"]}', 1, 1, 5, datetime('now')),
    ('opt-date-fmt', 'date_format', '"%Y-%m-%d"', 'select', 'general', '日期格式', NULL, '{"values":["%Y-%m-%d","%d/%m/%Y","%m/%d/%Y","%Y年%m月%d日"]}', 1, 1, 6, datetime('now'));

-- ── 阅读 ──────────────────────────────────────────────
INSERT INTO options (id, key, value, type, group_name, label, description, validation, is_public, autoload, sort_order, updated_at) VALUES
    ('opt-per-page', 'posts_per_page', '10', 'integer', 'reading', '每页文章数', NULL, '{"min":1,"max":100}', 1, 1, 10, datetime('now')),
    ('opt-rss-items', 'rss_items', '20', 'integer', 'reading', 'RSS 条目数', NULL, '{"min":1,"max":100}', 1, 1, 11, datetime('now')),
    ('opt-permalink', 'permalink_structure', '"/:year/:month/:slug"', 'select', 'reading', 'URL 结构', NULL, '{"values":["/:year/:month/:slug","/:slug","/posts/:slug"]}', 1, 1, 12, datetime('now'));

-- ── 讨论 ──────────────────────────────────────────────
INSERT INTO options (id, key, value, type, group_name, label, description, validation, is_public, autoload, sort_order, updated_at) VALUES
    ('opt-comment-mod', 'comment_moderation', 'true', 'boolean', 'discussion', '评论需审核', '开启后新评论需管理员审批', NULL, 0, 1, 20, datetime('now')),
    ('opt-comment-order', 'comment_order', '"asc"', 'select', 'discussion', '评论排序', NULL, '{"values":["asc","desc"]}', 1, 1, 21, datetime('now')),
    ('opt-default-role', 'default_role', '"reader"', 'select', 'discussion', '新用户默认角色', NULL, '{"values":["reader","author"]}', 0, 1, 22, datetime('now'));

-- ── 外观 ──────────────────────────────────────────────
INSERT INTO options (id, key, value, type, group_name, label, description, validation, is_public, autoload, sort_order, updated_at) VALUES
    ('opt-theme', 'theme', '"default"', 'select', 'appearance', '当前主题', NULL, '{"values":["default","corporate","minimal","warm"]}', 1, 1, 30, datetime('now')),
    ('opt-maintenance', 'maintenance_mode', 'false', 'boolean', 'appearance', '维护模式', '开启后前台显示维护页面', NULL, 1, 1, 31, datetime('now'));
