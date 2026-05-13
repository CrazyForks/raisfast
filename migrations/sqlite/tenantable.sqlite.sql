-- 内置表多租户支持（仅 BUILTIN_TENANTABLE=true 时由迁移执行器执行）

-- 租户表
CREATE TABLE IF NOT EXISTS tenants (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    document_id TEXT NOT NULL UNIQUE,
    name TEXT NOT NULL,
    domain TEXT UNIQUE,
    config TEXT NOT NULL DEFAULT '{}',
    status TEXT NOT NULL DEFAULT 'active',
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
);

-- 默认租户
INSERT OR IGNORE INTO tenants (document_id, name, domain, config, status, created_at, updated_at) VALUES
    ('default', 'Default', NULL, '{}', 'active', strftime('%Y-%m-%dT%H:%M:%SZ', 'now'), strftime('%Y-%m-%dT%H:%M:%SZ', 'now'));

-- 给所有内置业务表添加 tenant_id 列

-- 业务表
ALTER TABLE users ADD COLUMN tenant_id TEXT NOT NULL DEFAULT 'default';
ALTER TABLE posts ADD COLUMN tenant_id TEXT NOT NULL DEFAULT 'default';
ALTER TABLE categories ADD COLUMN tenant_id TEXT NOT NULL DEFAULT 'default';
ALTER TABLE tags ADD COLUMN tenant_id TEXT NOT NULL DEFAULT 'default';
ALTER TABLE comments ADD COLUMN tenant_id TEXT NOT NULL DEFAULT 'default';
ALTER TABLE media ADD COLUMN tenant_id TEXT NOT NULL DEFAULT 'default';
ALTER TABLE options ADD COLUMN tenant_id TEXT NOT NULL DEFAULT 'default';
ALTER TABLE pages ADD COLUMN tenant_id TEXT NOT NULL DEFAULT 'default';
ALTER TABLE reusable_blocks ADD COLUMN tenant_id TEXT NOT NULL DEFAULT 'default';

-- RBAC 表
ALTER TABLE roles ADD COLUMN tenant_id TEXT NOT NULL DEFAULT 'default';
ALTER TABLE permissions ADD COLUMN tenant_id TEXT NOT NULL DEFAULT 'default';

-- 审计 & Webhook
ALTER TABLE audit_log ADD COLUMN tenant_id TEXT NOT NULL DEFAULT 'default';
ALTER TABLE webhook_subscriptions ADD COLUMN tenant_id TEXT NOT NULL DEFAULT 'default';

-- Order system
ALTER TABLE products ADD COLUMN tenant_id TEXT NOT NULL DEFAULT 'default';
ALTER TABLE orders ADD COLUMN tenant_id TEXT NOT NULL DEFAULT 'default';
ALTER TABLE order_items ADD COLUMN tenant_id TEXT NOT NULL DEFAULT 'default';

-- Payment system
ALTER TABLE payment_channels ADD COLUMN tenant_id TEXT NOT NULL DEFAULT 'default';
ALTER TABLE payment_orders ADD COLUMN tenant_id TEXT NOT NULL DEFAULT 'default';
ALTER TABLE payment_transactions ADD COLUMN tenant_id TEXT NOT NULL DEFAULT 'default';
ALTER TABLE payment_refunds ADD COLUMN tenant_id TEXT NOT NULL DEFAULT 'default';

-- Wallet outbox
ALTER TABLE wallet_outbox ADD COLUMN tenant_id TEXT NOT NULL DEFAULT 'default';

-- Wallet system
ALTER TABLE wallets ADD COLUMN tenant_id TEXT NOT NULL DEFAULT 'default';
ALTER TABLE wallet_transactions ADD COLUMN tenant_id TEXT NOT NULL DEFAULT 'default';

-- 更新现有数据
UPDATE roles SET tenant_id = 'default';
UPDATE permissions SET tenant_id = 'default';
UPDATE options SET tenant_id = 'default' WHERE tenant_id IS NULL OR tenant_id = '';

-- 创建索引
CREATE INDEX IF NOT EXISTS idx_users_tenant ON users(tenant_id);
CREATE INDEX IF NOT EXISTS idx_posts_tenant ON posts(tenant_id);
CREATE INDEX IF NOT EXISTS idx_categories_tenant ON categories(tenant_id);
CREATE INDEX IF NOT EXISTS idx_tags_tenant ON tags(tenant_id);
CREATE INDEX IF NOT EXISTS idx_comments_tenant ON comments(tenant_id);
CREATE INDEX IF NOT EXISTS idx_media_tenant ON media(tenant_id);
CREATE INDEX IF NOT EXISTS idx_options_tenant_option_key ON options(tenant_id, option_key);
CREATE INDEX IF NOT EXISTS idx_pages_tenant_slug ON pages(tenant_id, slug);
CREATE INDEX IF NOT EXISTS idx_pages_tenant_status ON pages(tenant_id, status);
CREATE INDEX IF NOT EXISTS idx_reusable_blocks_tenant ON reusable_blocks(tenant_id);
CREATE INDEX IF NOT EXISTS idx_roles_tenant ON roles(tenant_id);
CREATE INDEX IF NOT EXISTS idx_permissions_tenant ON permissions(tenant_id);
CREATE INDEX IF NOT EXISTS idx_audit_log_tenant ON audit_log(tenant_id);
CREATE INDEX IF NOT EXISTS idx_webhook_subscriptions_tenant ON webhook_subscriptions(tenant_id);
CREATE INDEX IF NOT EXISTS idx_products_tenant ON products(tenant_id);
CREATE INDEX IF NOT EXISTS idx_orders_tenant ON orders(tenant_id);
CREATE INDEX IF NOT EXISTS idx_order_items_tenant ON order_items(tenant_id);
CREATE INDEX IF NOT EXISTS idx_payment_channels_tenant ON payment_channels(tenant_id);
CREATE INDEX IF NOT EXISTS idx_payment_orders_tenant ON payment_orders(tenant_id);
CREATE INDEX IF NOT EXISTS idx_payment_transactions_tenant ON payment_transactions(tenant_id);
CREATE INDEX IF NOT EXISTS idx_payment_refunds_tenant ON payment_refunds(tenant_id);
CREATE INDEX IF NOT EXISTS idx_wallet_outbox_tenant ON wallet_outbox(tenant_id);
CREATE INDEX IF NOT EXISTS idx_wallets_tenant ON wallets(tenant_id);
CREATE INDEX IF NOT EXISTS idx_wallet_transactions_tenant ON wallet_transactions(tenant_id);
