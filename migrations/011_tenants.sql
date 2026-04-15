-- 多租户基础
-- tenants 表 + 现有表加 tenant_id + rbac 租户隔离

-- 租户表
CREATE TABLE IF NOT EXISTS tenants (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    domain TEXT UNIQUE,
    config TEXT NOT NULL DEFAULT '{}',
    status TEXT NOT NULL DEFAULT 'active',
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

-- 预置默认租户
INSERT OR IGNORE INTO tenants (id, name, domain, config, status, created_at, updated_at) VALUES
    ('default', 'Default', NULL, '{}', 'active', datetime('now'), datetime('now'));

-- 给现有业务表加 tenant_id（带默认值 'default' 保持向后兼容）
ALTER TABLE users ADD COLUMN tenant_id TEXT NOT NULL DEFAULT 'default';
ALTER TABLE posts ADD COLUMN tenant_id TEXT NOT NULL DEFAULT 'default';
ALTER TABLE categories ADD COLUMN tenant_id TEXT NOT NULL DEFAULT 'default';
ALTER TABLE tags ADD COLUMN tenant_id TEXT NOT NULL DEFAULT 'default';
ALTER TABLE comments ADD COLUMN tenant_id TEXT NOT NULL DEFAULT 'default';
ALTER TABLE media ADD COLUMN tenant_id TEXT NOT NULL DEFAULT 'default';
-- options 表已在 009_options.sql v2 中包含 tenant_id

-- 给 rbac 表加 tenant_id（租户隔离）
ALTER TABLE roles ADD COLUMN tenant_id TEXT NOT NULL DEFAULT 'default';
ALTER TABLE permissions ADD COLUMN tenant_id TEXT NOT NULL DEFAULT 'default';

-- 更新现有 rbac 预置数据的 tenant_id
UPDATE roles SET tenant_id = 'default';
UPDATE permissions SET tenant_id = 'default';

-- 创建索引
CREATE INDEX IF NOT EXISTS idx_users_tenant ON users(tenant_id);
CREATE INDEX IF NOT EXISTS idx_posts_tenant ON posts(tenant_id);
CREATE INDEX IF NOT EXISTS idx_categories_tenant ON categories(tenant_id);
CREATE INDEX IF NOT EXISTS idx_tags_tenant ON tags(tenant_id);
CREATE INDEX IF NOT EXISTS idx_comments_tenant ON comments(tenant_id);
CREATE INDEX IF NOT EXISTS idx_media_tenant ON media(tenant_id);
CREATE INDEX IF NOT EXISTS idx_options_tenant_key ON options(tenant_id, key);
CREATE INDEX IF NOT EXISTS idx_roles_tenant ON roles(tenant_id);
CREATE INDEX IF NOT EXISTS idx_permissions_tenant ON permissions(tenant_id);
