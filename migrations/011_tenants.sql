-- 多租户基础
-- tenants 表
-- 内置表 tenant_id 列由 026_builtin_tenantable.sql 添加（仅 BUILTIN_TENANTABLE=true）

CREATE TABLE IF NOT EXISTS tenants (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    domain TEXT UNIQUE,
    config TEXT NOT NULL DEFAULT '{}',
    status TEXT NOT NULL DEFAULT 'active',
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

INSERT OR IGNORE INTO tenants (id, name, domain, config, status, created_at, updated_at) VALUES
    ('default', 'Default', NULL, '{}', 'active', datetime('now'), datetime('now'));
