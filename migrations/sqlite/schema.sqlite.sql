-- ============================================================
-- raisfast 完整数据库 Schema（BUILTIN_TENANTABLE=false 默认模式）
-- 由所有 migration 文件合并而成，用于新部署一键初始化
-- 生成日期：2026-05-07
--
-- 注意：此 schema 不含 tenant_id 列。
-- 若需多租户支持，设置 BUILTIN_TENANTABLE=true 后迁移 026 会自动添加。
-- ============================================================

-- ── 平台基础层（永不禁用） ──────────────────────────────────

-- 用户
CREATE TABLE IF NOT EXISTS users (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    document_id TEXT NOT NULL UNIQUE,
    username TEXT UNIQUE NOT NULL,
    role TEXT NOT NULL DEFAULT 'reader',
    avatar TEXT,
    bio TEXT,
    website TEXT,
    status TEXT NOT NULL DEFAULT 'active',
    registered_via TEXT NOT NULL,
    display_name TEXT,
    slug TEXT UNIQUE,
    locale TEXT,
    social_links TEXT,
    metadata TEXT,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
);

CREATE INDEX IF NOT EXISTS idx_users_username ON users(username);
CREATE UNIQUE INDEX IF NOT EXISTS idx_users_slug ON users(slug) WHERE slug IS NOT NULL;

-- 用户凭据
CREATE TABLE IF NOT EXISTS user_credentials (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    document_id TEXT NOT NULL UNIQUE,
    user_id INTEGER NOT NULL REFERENCES users(id),
    auth_type TEXT NOT NULL,
    identifier TEXT NOT NULL,
    credential_data TEXT NOT NULL,
    verified INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    UNIQUE(auth_type, identifier)
);

CREATE INDEX IF NOT EXISTS idx_user_credentials_user ON user_credentials(user_id);
CREATE INDEX IF NOT EXISTS idx_user_credentials_type_id ON user_credentials(auth_type, identifier);
CREATE INDEX IF NOT EXISTS idx_user_credentials_type ON user_credentials(auth_type);

-- OAuth 账号绑定
CREATE TABLE IF NOT EXISTS oauth_accounts (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    document_id TEXT NOT NULL UNIQUE,
    user_id INTEGER NOT NULL REFERENCES users(id),
    provider TEXT NOT NULL,
    provider_user_id TEXT NOT NULL,
    email TEXT,
    display_name TEXT,
    avatar_url TEXT,
    access_token TEXT,
    refresh_token TEXT,
    token_expires_at TEXT,
    profile TEXT,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    UNIQUE(provider, provider_user_id)
);

CREATE INDEX IF NOT EXISTS idx_oauth_accounts_user ON oauth_accounts(user_id);
CREATE INDEX IF NOT EXISTS idx_oauth_accounts_provider ON oauth_accounts(provider, provider_user_id);

-- OAuth 短期 state 存储（PKCE）
CREATE TABLE IF NOT EXISTS oauth_states (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    document_id TEXT NOT NULL UNIQUE,
    provider TEXT NOT NULL,
    code_verifier TEXT NOT NULL,
    user_id INTEGER,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    expires_at TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_oauth_states_expires ON oauth_states(expires_at);

-- 币种配置
CREATE TABLE IF NOT EXISTS currencies (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    document_id TEXT NOT NULL UNIQUE,
    code TEXT NOT NULL UNIQUE CHECK(code = UPPER(code) AND LENGTH(code) BETWEEN 1 AND 10),
    name TEXT NOT NULL,
    decimals INTEGER NOT NULL DEFAULT 0 CHECK(decimals BETWEEN 0 AND 18),
    is_active INTEGER NOT NULL DEFAULT 1,
    version INTEGER NOT NULL DEFAULT 1,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_currencies_code ON currencies(code);

-- 用户钱包（每用户每币种一个）
CREATE TABLE IF NOT EXISTS wallets (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    document_id TEXT NOT NULL UNIQUE,
    user_id INTEGER NOT NULL REFERENCES users(id),
    currency TEXT NOT NULL,
    balance INTEGER NOT NULL DEFAULT 0 CHECK(balance >= 0),
    version INTEGER NOT NULL DEFAULT 1,
    status TEXT NOT NULL DEFAULT 'active',
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    UNIQUE(user_id, currency)
);

CREATE INDEX IF NOT EXISTS idx_wallets_user ON wallets(user_id);
CREATE INDEX IF NOT EXISTS idx_wallets_currency ON wallets(currency);

-- 不可变交易流水
CREATE TABLE IF NOT EXISTS wallet_transactions (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    document_id TEXT NOT NULL UNIQUE,
    wallet_id INTEGER NOT NULL REFERENCES wallets(id),
    user_id INTEGER NOT NULL REFERENCES users(id),
    entry_type TEXT NOT NULL,
    amount INTEGER NOT NULL CHECK(amount > 0),
    balance_after INTEGER NOT NULL CHECK(balance_after >= 0),
    tx_type TEXT NOT NULL,
    currency TEXT NOT NULL,
    transaction_no TEXT NOT NULL UNIQUE,
    related_tx_id INTEGER,
    reference_type TEXT,
    reference_id TEXT,
    counterparty_wallet_id INTEGER,
    metadata TEXT,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
);

CREATE INDEX IF NOT EXISTS idx_wallet_tx_wallet ON wallet_transactions(wallet_id);
CREATE INDEX IF NOT EXISTS idx_wallet_tx_user ON wallet_transactions(user_id, created_at DESC);
CREATE INDEX IF NOT EXISTS idx_wallet_tx_transaction_no ON wallet_transactions(transaction_no);
CREATE INDEX IF NOT EXISTS idx_wallet_tx_tx_type ON wallet_transactions(tx_type);
CREATE INDEX IF NOT EXISTS idx_wallet_tx_reference ON wallet_transactions(reference_type, reference_id);
CREATE INDEX IF NOT EXISTS idx_wallet_tx_created ON wallet_transactions(created_at);

-- Refresh Tokens
CREATE TABLE IF NOT EXISTS refresh_tokens (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    document_id TEXT NOT NULL UNIQUE,
    user_id INTEGER NOT NULL REFERENCES users(id),
    token TEXT UNIQUE NOT NULL,
    expires_at TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
);

CREATE INDEX IF NOT EXISTS idx_refresh_tokens_user ON refresh_tokens(user_id);
CREATE INDEX IF NOT EXISTS idx_refresh_tokens_token ON refresh_tokens(token);
CREATE INDEX IF NOT EXISTS idx_refresh_tokens_expires_at ON refresh_tokens(expires_at);

-- 站点配置
CREATE TABLE IF NOT EXISTS options (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    document_id TEXT NOT NULL UNIQUE,
    option_key TEXT NOT NULL,
    value TEXT NOT NULL,
    type TEXT NOT NULL DEFAULT 'text',
    group_name TEXT NOT NULL DEFAULT 'general',
    label TEXT NOT NULL DEFAULT '',
    description TEXT,
    validation TEXT,
    is_public BOOLEAN NOT NULL DEFAULT 0,
    autoload BOOLEAN NOT NULL DEFAULT 1,
    sort_order INTEGER NOT NULL DEFAULT 0,
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    UNIQUE(option_key)
);

-- RBAC 角色
CREATE TABLE IF NOT EXISTS roles (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    document_id TEXT NOT NULL UNIQUE,
    name TEXT NOT NULL UNIQUE,
    description TEXT,
    is_system BOOLEAN NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
);

-- RBAC 权限
CREATE TABLE IF NOT EXISTS permissions (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    document_id TEXT NOT NULL UNIQUE,
    role_id INTEGER NOT NULL REFERENCES roles(id),
    action TEXT NOT NULL,
    subject TEXT NOT NULL,
     fields TEXT,
    conditions TEXT,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
);


CREATE UNIQUE INDEX IF NOT EXISTS idx_permissions_role_action_subject
    ON permissions(role_id, action, subject);

-- 租户
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

-- 审计日志
CREATE TABLE IF NOT EXISTS audit_log (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    document_id TEXT NOT NULL UNIQUE,
    actor_id INTEGER,
    actor_role TEXT,
    action TEXT NOT NULL,
    subject TEXT NOT NULL,
    subject_id TEXT,
    detail TEXT,
     ip_address TEXT,
    user_agent TEXT,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
);


CREATE INDEX IF NOT EXISTS idx_audit_log_action ON audit_log(action);
CREATE INDEX IF NOT EXISTS idx_audit_log_actor ON audit_log(actor_id);
CREATE INDEX IF NOT EXISTS idx_audit_log_created ON audit_log(created_at);

-- API Token
CREATE TABLE IF NOT EXISTS api_tokens (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    document_id TEXT NOT NULL UNIQUE,
    user_id INTEGER NOT NULL REFERENCES users(id),
    name TEXT NOT NULL,
    token_hash TEXT UNIQUE NOT NULL,
    token_prefix TEXT NOT NULL,
    scopes TEXT NOT NULL DEFAULT '["read","write"]',
    last_used_at TEXT,
    expires_at TEXT,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
);

CREATE INDEX IF NOT EXISTS idx_api_tokens_user_id ON api_tokens(user_id);
CREATE INDEX IF NOT EXISTS idx_api_tokens_token_hash ON api_tokens(token_hash);

-- Webhook 订阅
CREATE TABLE IF NOT EXISTS webhook_subscriptions (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    document_id TEXT NOT NULL UNIQUE,
    url TEXT NOT NULL,
    secret TEXT NOT NULL,
    events TEXT NOT NULL DEFAULT '[]',
    enabled INTEGER NOT NULL DEFAULT 1,
    description TEXT,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
);

CREATE INDEX IF NOT EXISTS idx_webhook_subscriptions_enabled ON webhook_subscriptions(enabled);

-- 插件 KV 存储
CREATE TABLE IF NOT EXISTS plugin_storage (
    plugin_id TEXT NOT NULL,
    storage_key TEXT NOT NULL,
    value TEXT NOT NULL,
    expires_at TEXT,
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    PRIMARY KEY (plugin_id, storage_key)
);

CREATE INDEX IF NOT EXISTS idx_plugin_storage_plugin ON plugin_storage(plugin_id);

-- 内容版本历史
CREATE TABLE IF NOT EXISTS content_revisions (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    document_id TEXT NOT NULL UNIQUE,
    content_type TEXT NOT NULL,
    record_id TEXT NOT NULL,
    revision_number INTEGER NOT NULL,
    snapshot TEXT NOT NULL,
    created_by INTEGER,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    UNIQUE(content_type, record_id, revision_number)
);

CREATE INDEX IF NOT EXISTS idx_revisions_ct_record
    ON content_revisions(content_type, record_id);
CREATE INDEX IF NOT EXISTS idx_revisions_ct_record_rev
    ON content_revisions(content_type, record_id, revision_number DESC);

-- 密码重置令牌
CREATE TABLE IF NOT EXISTS password_reset_tokens (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    document_id TEXT NOT NULL UNIQUE,
    user_id INTEGER NOT NULL REFERENCES users(id),
    token TEXT NOT NULL UNIQUE,
    expires_at TEXT NOT NULL,
    used_at TEXT,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
);

CREATE INDEX IF NOT EXISTS idx_password_reset_tokens_token ON password_reset_tokens(token);
CREATE INDEX IF NOT EXISTS idx_password_reset_tokens_user_id ON password_reset_tokens(user_id);
CREATE INDEX IF NOT EXISTS idx_password_reset_tokens_expires_at ON password_reset_tokens(expires_at);

-- 短信验证码
CREATE TABLE IF NOT EXISTS sms_codes (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    document_id TEXT NOT NULL UNIQUE,
    phone TEXT NOT NULL,
    code TEXT NOT NULL,
    purpose TEXT NOT NULL,
    expires_at TEXT NOT NULL,
    verified_at TEXT,
    attempts INTEGER NOT NULL DEFAULT 0,
    ip_address TEXT,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
);

CREATE INDEX IF NOT EXISTS idx_sms_codes_phone ON sms_codes(phone);
CREATE INDEX IF NOT EXISTS idx_sms_codes_expires ON sms_codes(expires_at);

-- 邮箱验证令牌
CREATE TABLE IF NOT EXISTS email_verification_tokens (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    document_id TEXT NOT NULL UNIQUE,
    user_id INTEGER NOT NULL REFERENCES users(id),
    token TEXT NOT NULL UNIQUE,
    email TEXT NOT NULL,
    expires_at TEXT NOT NULL,
    verified_at TEXT,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
);

CREATE INDEX IF NOT EXISTS idx_email_verification_tokens_token ON email_verification_tokens(token);
CREATE INDEX IF NOT EXISTS idx_email_verification_tokens_user_id ON email_verification_tokens(user_id);
CREATE INDEX IF NOT EXISTS idx_email_verification_tokens_expires ON email_verification_tokens(expires_at);

-- 后台任务队列
CREATE TABLE IF NOT EXISTS jobs (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    document_id  TEXT NOT NULL UNIQUE,
    job_type     TEXT NOT NULL,
    payload      TEXT NOT NULL,
    status       TEXT NOT NULL DEFAULT 'pending',
    attempts     INTEGER NOT NULL DEFAULT 0,
    max_attempts INTEGER NOT NULL DEFAULT 3,
    run_after    TEXT,
    error        TEXT,
    created_at   TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    updated_at   TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
);

CREATE INDEX IF NOT EXISTS idx_jobs_status ON jobs(status);
CREATE INDEX IF NOT EXISTS idx_jobs_run_after ON jobs(run_after) WHERE status = 'pending';
CREATE INDEX IF NOT EXISTS idx_jobs_type ON jobs(job_type);

-- 定时任务调度
CREATE TABLE IF NOT EXISTS cron_schedules (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    document_id  TEXT NOT NULL UNIQUE,
    label        TEXT NOT NULL,
    job_type     TEXT NOT NULL,
    payload      TEXT,
    cron_expr    TEXT NOT NULL,
    enabled      INTEGER NOT NULL DEFAULT 1,
    last_run_at  TEXT,
    next_run_at  TEXT NOT NULL,
    plugin_id    TEXT,
    created_at   TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    updated_at   TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
);

CREATE INDEX IF NOT EXISTS idx_cron_enabled ON cron_schedules(enabled);
CREATE INDEX IF NOT EXISTS idx_cron_next_run ON cron_schedules(next_run_at) WHERE enabled = 1;
CREATE INDEX IF NOT EXISTS idx_cron_plugin ON cron_schedules(plugin_id);

-- Cron 执行历史
CREATE TABLE IF NOT EXISTS cron_execution_log (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    document_id  TEXT NOT NULL UNIQUE,
    schedule_id  INTEGER NOT NULL,
    job_type     TEXT NOT NULL,
    label        TEXT NOT NULL,
    status       TEXT NOT NULL DEFAULT 'running',
    duration_ms  INTEGER,
    error        TEXT,
    started_at   TEXT NOT NULL,
    finished_at  TEXT
);

CREATE INDEX IF NOT EXISTS idx_cron_log_schedule ON cron_execution_log(schedule_id);
CREATE INDEX IF NOT EXISTS idx_cron_log_status ON cron_execution_log(status);
CREATE INDEX IF NOT EXISTS idx_cron_log_started ON cron_execution_log(started_at);

-- ── 内置模块：Blog（BUILTIN_BLOG=true） ──────────────────

-- 分类
CREATE TABLE IF NOT EXISTS categories (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    document_id TEXT NOT NULL UNIQUE,
    name TEXT UNIQUE NOT NULL,
    slug TEXT UNIQUE NOT NULL,
    description TEXT,
    parent_id INTEGER REFERENCES categories(id),
    sort_order INTEGER NOT NULL DEFAULT 0,
    created_by INTEGER,
    updated_by INTEGER,
    cover_image TEXT,
    meta_title TEXT,
    meta_description TEXT,
    og_title TEXT,
    og_description TEXT,
    og_image TEXT,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
);

-- 标签
CREATE TABLE IF NOT EXISTS tags (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    document_id TEXT NOT NULL UNIQUE,
    name TEXT UNIQUE NOT NULL,
    slug TEXT UNIQUE NOT NULL,
    created_by INTEGER,
    updated_by INTEGER,
    description TEXT,
    cover_image TEXT,
    meta_title TEXT,
    meta_description TEXT,
    og_title TEXT,
    og_description TEXT,
    og_image TEXT,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
);

-- 文章
CREATE TABLE IF NOT EXISTS posts (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    document_id TEXT NOT NULL UNIQUE,
    title TEXT NOT NULL,
    slug TEXT UNIQUE NOT NULL,
    content TEXT NOT NULL,
    excerpt TEXT,
    cover_image TEXT,
    status TEXT NOT NULL DEFAULT 'draft',
    created_by INTEGER NOT NULL REFERENCES users(id),
    updated_by INTEGER,
    category_id INTEGER REFERENCES categories(id),
    view_count INTEGER NOT NULL DEFAULT 0,
    is_pinned INTEGER NOT NULL DEFAULT 0,
    password TEXT,
    comment_status TEXT NOT NULL DEFAULT 'open',
    format TEXT NOT NULL DEFAULT 'standard',
    template TEXT NOT NULL DEFAULT 'default',
    meta_title TEXT,
    meta_description TEXT,
    og_title TEXT,
    og_description TEXT,
    og_image TEXT,
    canonical_url TEXT,
    reading_time INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    published_at TEXT
);

CREATE INDEX IF NOT EXISTS idx_posts_slug ON posts(slug);
CREATE INDEX IF NOT EXISTS idx_posts_status ON posts(status);
CREATE INDEX IF NOT EXISTS idx_posts_author ON posts(created_by);
CREATE INDEX IF NOT EXISTS idx_posts_category ON posts(category_id);
CREATE INDEX IF NOT EXISTS idx_posts_created ON posts(created_at);
CREATE INDEX IF NOT EXISTS idx_posts_status_created
    ON posts(status, is_pinned DESC, created_at DESC);
CREATE INDEX IF NOT EXISTS idx_posts_status_category
    ON posts(status, category_id);
CREATE INDEX IF NOT EXISTS idx_posts_status_author
    ON posts(status, created_by);

-- 文章-标签（多对多）
CREATE TABLE IF NOT EXISTS posts_tags (
    post_id INTEGER NOT NULL REFERENCES posts(id),
    tag_id INTEGER NOT NULL REFERENCES tags(id),
    PRIMARY KEY (post_id, tag_id)
);

CREATE INDEX IF NOT EXISTS idx_posts_tags_tag_id ON posts_tags(tag_id);

-- 评论
CREATE TABLE IF NOT EXISTS comments (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    document_id TEXT NOT NULL UNIQUE,
    post_id INTEGER NOT NULL REFERENCES posts(id),
    created_by INTEGER REFERENCES users(id),
    updated_by INTEGER,
    nickname TEXT,
    email TEXT,
    content TEXT NOT NULL,
    parent_id INTEGER REFERENCES comments(id),
    status TEXT NOT NULL DEFAULT 'pending',
    author_ip TEXT,
    author_url TEXT,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
);

CREATE INDEX IF NOT EXISTS idx_comments_post ON comments(post_id);
CREATE INDEX IF NOT EXISTS idx_comments_status ON comments(status);
CREATE INDEX IF NOT EXISTS idx_comments_post_status
    ON comments(post_id, status);
CREATE INDEX IF NOT EXISTS idx_comments_parent_id
    ON comments(parent_id);

-- ── 内置模块：Pages（BUILTIN_PAGES=true） ────────────────

CREATE TABLE IF NOT EXISTS pages (
    id               INTEGER PRIMARY KEY AUTOINCREMENT,
    document_id      TEXT NOT NULL UNIQUE,
    title            TEXT NOT NULL,
    slug             TEXT NOT NULL UNIQUE,
    content          TEXT,
    blocks           TEXT,
    meta_title       TEXT,
    meta_description TEXT,
    og_image         TEXT,
    template         TEXT NOT NULL DEFAULT 'default',
    parent_id        INTEGER REFERENCES pages(id),
    sort_order       INTEGER NOT NULL DEFAULT 0,
    status           TEXT NOT NULL DEFAULT 'draft',
    created_by       INTEGER NOT NULL REFERENCES users(id),
    updated_by       INTEGER,
    cover_image      TEXT,
    published_at     TEXT,
    password TEXT,
    comment_status TEXT NOT NULL DEFAULT 'closed',
    og_title TEXT,
    og_description TEXT,
    canonical_url TEXT,
    created_at       TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    updated_at       TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
);

CREATE INDEX IF NOT EXISTS idx_pages_slug      ON pages(slug);
CREATE INDEX IF NOT EXISTS idx_pages_status    ON pages(status);
CREATE INDEX IF NOT EXISTS idx_pages_parent    ON pages(parent_id);
CREATE INDEX IF NOT EXISTS idx_pages_author    ON pages(created_by);

CREATE TABLE IF NOT EXISTS reusable_blocks (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    document_id TEXT NOT NULL UNIQUE,
    name        TEXT NOT NULL,
    block_type  TEXT NOT NULL,
    content     TEXT NOT NULL,
    description TEXT,
    created_by  INTEGER,
    updated_by  INTEGER,
    created_at  TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    updated_at  TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
);

-- ── 内置模块：Media（BUILTIN_MEDIA=true） ────────────────

CREATE TABLE IF NOT EXISTS media (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    document_id TEXT NOT NULL UNIQUE,
    user_id INTEGER NOT NULL REFERENCES users(id),
    filename TEXT NOT NULL,
    filepath TEXT NOT NULL,
    mimetype TEXT NOT NULL,
    size INTEGER NOT NULL,
    width INTEGER,
    height INTEGER,
    title TEXT,
    alt_text TEXT,
    caption TEXT,
    description TEXT,
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
);

CREATE INDEX IF NOT EXISTS idx_media_user_created
    ON media(user_id, created_at DESC);

-- ── 内置模块：Workflow（BUILTIN_WORKFLOW=true） ──────────

CREATE TABLE IF NOT EXISTS workflow_definitions (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    document_id TEXT NOT NULL UNIQUE,
    name TEXT NOT NULL,
    description TEXT,
    steps TEXT NOT NULL,
    initial_step TEXT NOT NULL,
    version INTEGER NOT NULL DEFAULT 1,
    enabled INTEGER NOT NULL DEFAULT 1,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
);

CREATE TABLE IF NOT EXISTS workflow_instances (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    document_id TEXT NOT NULL UNIQUE,
    definition_id INTEGER NOT NULL REFERENCES workflow_definitions(id),
    status TEXT NOT NULL DEFAULT 'running',
    current_step TEXT,
    context TEXT NOT NULL DEFAULT '{}',
    triggered_by INTEGER,
    started_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    completed_at TEXT,
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
);

CREATE INDEX IF NOT EXISTS idx_wf_instances_definition ON workflow_instances(definition_id);
CREATE INDEX IF NOT EXISTS idx_wf_instances_status ON workflow_instances(status);

CREATE TABLE IF NOT EXISTS workflow_step_logs (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    document_id TEXT NOT NULL UNIQUE,
    instance_id INTEGER NOT NULL REFERENCES workflow_instances(id),
    step_id TEXT NOT NULL,
    step_name TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'running',
    input TEXT,
    output TEXT,
    error TEXT,
    started_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    completed_at TEXT
);

CREATE INDEX IF NOT EXISTS idx_wf_step_logs_instance ON workflow_step_logs(instance_id);

-- ============================================================
-- 预置数据
-- ============================================================

-- 默认租户
INSERT OR IGNORE INTO tenants (document_id, name, domain, config, status, created_at, updated_at) VALUES
    ('default', 'Default', NULL, '{}', 'active', strftime('%Y-%m-%dT%H:%M:%SZ', 'now'), strftime('%Y-%m-%dT%H:%M:%SZ', 'now'));

-- 系统角色
INSERT OR IGNORE INTO roles (document_id, name, description, is_system, created_at, updated_at) VALUES
    ('role-admin', 'admin', '超级管理员', 1, strftime('%Y-%m-%dT%H:%M:%SZ', 'now'), strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    ('role-editor', 'editor', '编辑', 0, strftime('%Y-%m-%dT%H:%M:%SZ', 'now'), strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    ('role-author', 'author', '作者', 0, strftime('%Y-%m-%dT%H:%M:%SZ', 'now'), strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    ('role-reader', 'reader', '读者', 1, strftime('%Y-%m-%dT%H:%M:%SZ', 'now'), strftime('%Y-%m-%dT%H:%M:%SZ', 'now'));

-- admin 全局权限
INSERT OR IGNORE INTO permissions (document_id, role_id, action, subject, fields, conditions, created_at) VALUES
    ('perm-admin-all', (SELECT id FROM roles WHERE document_id = 'role-admin'), '*', '*', '["*"]', NULL, strftime('%Y-%m-%dT%H:%M:%SZ', 'now'));

-- editor 权限
INSERT OR IGNORE INTO permissions (document_id, role_id, action, subject, fields, conditions, created_at) VALUES
    ('perm-editor-ct-create', (SELECT id FROM roles WHERE document_id = 'role-editor'), 'content-type::*.*', 'content-type::*', '["*"]', NULL, strftime('%Y-%m-%dT%H:%M:%SZ', 'now'));

-- author 权限
INSERT OR IGNORE INTO permissions (document_id, role_id, action, subject, fields, conditions, created_at) VALUES
    ('perm-author-post-create', (SELECT id FROM roles WHERE document_id = 'role-author'), 'content-type::post.create', 'content-type::post', '["*"]', NULL, strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    ('perm-author-post-read', (SELECT id FROM roles WHERE document_id = 'role-author'), 'content-type::post.read', 'content-type::post', '["*"]', NULL, strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    ('perm-author-post-update', (SELECT id FROM roles WHERE document_id = 'role-author'), 'content-type::post.update', 'content-type::post', '["*"]', '{"author_id":"$user.id"}', strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    ('perm-author-post-delete', (SELECT id FROM roles WHERE document_id = 'role-author'), 'content-type::post.delete', 'content-type::post', '["*"]', '{"author_id":"$user.id"}', strftime('%Y-%m-%dT%H:%M:%SZ', 'now'));

-- reader 权限
INSERT OR IGNORE INTO permissions (document_id, role_id, action, subject, fields, conditions, created_at) VALUES
    ('perm-reader-post-read', (SELECT id FROM roles WHERE document_id = 'role-reader'), 'content-type::post.read', 'content-type::post', '["title","slug","content","excerpt","status"]', NULL, strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    ('perm-reader-comment-create', (SELECT id FROM roles WHERE document_id = 'role-reader'), 'content-type::comment.create', 'content-type::comment', '["content","nickname","email"]', NULL, strftime('%Y-%m-%dT%H:%M:%SZ', 'now'));

-- 站点配置
INSERT OR IGNORE INTO options (document_id, option_key, value, type, group_name, label, description, validation, is_public, autoload, sort_order, updated_at) VALUES
    ('opt-site-title', 'site_title', '"My Blog"', 'text', 'general', '站点标题', '显示在浏览器标题栏和页面头部', '{"max_length":100}', 1, 1, 1, strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    ('opt-site-desc', 'site_description', '""', 'text', 'general', '站点描述', '简短描述站点用途', '{"max_length":500}', 1, 1, 2, strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    ('opt-site-url', 'site_url', '""', 'url', 'general', '站点 URL', '如 https://example.com', NULL, 1, 1, 3, strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    ('opt-admin-email', 'admin_email', '""', 'email', 'general', '管理员邮箱', NULL, NULL, 0, 1, 4, strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    ('opt-timezone', 'timezone', '"UTC"', 'select', 'general', '时区', NULL, '{"values":["UTC","Asia/Shanghai","Asia/Tokyo","US/Eastern","US/Pacific","Europe/London","Europe/Berlin"]}', 1, 1, 5, strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    ('opt-date-fmt', 'date_format', '"%Y-%m-%d"', 'select', 'general', '日期格式', NULL, '{"values":["%Y-%m-%d","%d/%m/%Y","%m/%d/%Y","%Y年%m月%d日"]}', 1, 1, 6, strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    ('opt-per-page', 'posts_per_page', '10', 'integer', 'reading', '每页文章数', NULL, '{"min":1,"max":100}', 1, 1, 10, strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    ('opt-rss-items', 'rss_items', '20', 'integer', 'reading', 'RSS 条目数', NULL, '{"min":1,"max":100}', 1, 1, 11, strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    ('opt-permalink', 'permalink_structure', '"/:year/:month/:slug"', 'select', 'reading', 'URL 结构', NULL, '{"values":["/:year/:month/:slug","/:slug","/posts/:slug"]}', 1, 1, 12, strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    ('opt-comment-mod', 'comment_moderation', 'true', 'boolean', 'discussion', '评论需审核', '开启后新评论需管理员审批', NULL, 0, 1, 20, strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    ('opt-comment-order', 'comment_order', '"asc"', 'select', 'discussion', '评论排序', NULL, '{"values":["asc","desc"]}', 1, 1, 21, strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    ('opt-default-role', 'default_role', '"reader"', 'select', 'discussion', '新用户默认角色', NULL, '{"values":["reader","author"]}', 0, 1, 22, strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    ('opt-theme', 'theme', '"default"', 'select', 'appearance', '当前主题', NULL, '{"values":["default","corporate","minimal","warm"]}', 1, 1, 30, strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    ('opt-maintenance', 'maintenance_mode', 'false', 'boolean', 'appearance', '维护模式', '开启后前台显示维护页面', NULL, 1, 1, 31, strftime('%Y-%m-%dT%H:%M:%SZ', 'now'));

-- Products
CREATE TABLE IF NOT EXISTS products (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    document_id TEXT NOT NULL UNIQUE,
    category_id INTEGER REFERENCES categories(id),
    title TEXT NOT NULL,
    description TEXT,
    cover_url TEXT,
    product_type TEXT NOT NULL DEFAULT 'custom',
    fulfillment_type TEXT NOT NULL DEFAULT 'digital',
    delivery_hook TEXT,
    weight INTEGER,
    shipping_template_id INTEGER,
    price INTEGER NOT NULL CHECK(price >= 0),
    currency TEXT NOT NULL DEFAULT 'USD',
    status TEXT NOT NULL DEFAULT 'draft',
    attributes TEXT,
    sort_order INTEGER NOT NULL DEFAULT 0,
    version INTEGER NOT NULL DEFAULT 1,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    slug TEXT UNIQUE,
    content TEXT,
    image_ids TEXT,
    original_price INTEGER,
    specs TEXT,
    unit TEXT NOT NULL DEFAULT 'piece',
    min_purchase INTEGER NOT NULL DEFAULT 1,
    max_purchase INTEGER,
    total_sales INTEGER NOT NULL DEFAULT 0,
    virtual_sales INTEGER NOT NULL DEFAULT 0,
    meta_title TEXT,
    meta_description TEXT,
    published_at TEXT,
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
);

CREATE INDEX IF NOT EXISTS idx_products_status ON products(status);
CREATE INDEX IF NOT EXISTS idx_products_type ON products(product_type);
CREATE INDEX IF NOT EXISTS idx_products_slug ON products(slug);

-- Orders
CREATE TABLE IF NOT EXISTS orders (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    document_id TEXT NOT NULL UNIQUE,
    user_id INTEGER NOT NULL REFERENCES users(id),
    order_no TEXT NOT NULL UNIQUE,
    subtotal INTEGER NOT NULL DEFAULT 0,
    discount_amount INTEGER NOT NULL DEFAULT 0,
    shipping_amount INTEGER NOT NULL DEFAULT 0,
    total_amount INTEGER NOT NULL CHECK(total_amount >= 0),
    currency TEXT NOT NULL DEFAULT 'USD',
    status TEXT NOT NULL DEFAULT 'pending',
    buyer_name TEXT,
    buyer_phone TEXT,
    buyer_email TEXT,
    shipping_address TEXT,
    tracking_no TEXT,
    carrier TEXT,
    remark TEXT,
    admin_remark TEXT,
    delivery_data TEXT,
    paid_at TEXT,
    completed_at TEXT,
    cancelled_at TEXT,
    refunding_at TEXT,
    refunded_at TEXT,
    expired_at TEXT,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
);

CREATE INDEX IF NOT EXISTS idx_orders_user ON orders(user_id);
CREATE INDEX IF NOT EXISTS idx_orders_status ON orders(status);
CREATE INDEX IF NOT EXISTS idx_orders_order_no ON orders(order_no);

-- Order Items
CREATE TABLE IF NOT EXISTS order_items (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    document_id TEXT NOT NULL UNIQUE,
    order_id INTEGER NOT NULL REFERENCES orders(id),
    product_id INTEGER REFERENCES products(id),
    title TEXT NOT NULL,
    description TEXT,
    unit_price INTEGER NOT NULL CHECK(unit_price >= 0),
    quantity INTEGER NOT NULL CHECK(quantity > 0),
    subtotal INTEGER NOT NULL,
    cover_url TEXT,
    attributes TEXT,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
);

CREATE INDEX IF NOT EXISTS idx_order_items_order ON order_items(order_id);

-- Payment Channels
CREATE TABLE IF NOT EXISTS payment_channels (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    document_id TEXT NOT NULL UNIQUE,
    provider TEXT NOT NULL,
    name TEXT NOT NULL,
    is_live INTEGER NOT NULL DEFAULT 0,
    credentials TEXT NOT NULL,
    webhook_secret TEXT,
    settings TEXT,
    is_active INTEGER NOT NULL DEFAULT 1,
    sort_order INTEGER NOT NULL DEFAULT 0,
    version INTEGER NOT NULL DEFAULT 1,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    UNIQUE(provider, name)
);

CREATE INDEX IF NOT EXISTS idx_payment_channels_provider ON payment_channels(provider);
CREATE INDEX IF NOT EXISTS idx_payment_channels_active ON payment_channels(is_active);

-- Payment Orders
CREATE TABLE IF NOT EXISTS payment_orders (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    document_id TEXT NOT NULL UNIQUE,
    user_id INTEGER NOT NULL REFERENCES users(id),
    order_id TEXT,
    title TEXT NOT NULL,
    amount INTEGER NOT NULL,
    currency TEXT NOT NULL DEFAULT 'USD',
    channel_id INTEGER NOT NULL REFERENCES payment_channels(id),
    provider TEXT NOT NULL,
    provider_order_id TEXT,
    provider_method TEXT,
    status TEXT NOT NULL DEFAULT 'pending',
    reference_type TEXT,
    reference_id TEXT,
    return_url TEXT,
    idempotency_key TEXT NOT NULL UNIQUE,
    version INTEGER NOT NULL DEFAULT 1,
    provider_data TEXT,
    client_ip TEXT,
    metadata TEXT,
    paid_at TEXT,
    cancelled_at TEXT,
    expired_at TEXT,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
);

CREATE INDEX IF NOT EXISTS idx_payment_orders_user ON payment_orders(user_id);
CREATE INDEX IF NOT EXISTS idx_payment_orders_status ON payment_orders(status);
CREATE INDEX IF NOT EXISTS idx_payment_orders_provider ON payment_orders(provider_order_id);
CREATE INDEX IF NOT EXISTS idx_payment_orders_order_id ON payment_orders(order_id);

-- Payment Transactions
CREATE TABLE IF NOT EXISTS payment_transactions (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    document_id TEXT NOT NULL UNIQUE,
    payment_order_id INTEGER NOT NULL REFERENCES payment_orders(id),
    order_id TEXT,
    user_id INTEGER NOT NULL REFERENCES users(id),
    tx_type TEXT NOT NULL,
    amount INTEGER NOT NULL,
    currency TEXT NOT NULL,
    provider_tx_id TEXT NOT NULL UNIQUE,
    status TEXT NOT NULL DEFAULT 'pending',
    raw_payload TEXT,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
);

CREATE INDEX IF NOT EXISTS idx_payment_tx_order ON payment_transactions(payment_order_id);
CREATE INDEX IF NOT EXISTS idx_payment_tx_order_id ON payment_transactions(order_id);

-- Payment Refunds
CREATE TABLE IF NOT EXISTS payment_refunds (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    document_id TEXT NOT NULL UNIQUE,
    payment_order_id INTEGER NOT NULL REFERENCES payment_orders(id),
    order_id TEXT,
    user_id INTEGER NOT NULL REFERENCES users(id),
    amount INTEGER NOT NULL,
    currency TEXT NOT NULL,
    reason TEXT,
    provider_refund_id TEXT,
    status TEXT NOT NULL DEFAULT 'pending',
    payment_tx_id INTEGER REFERENCES payment_transactions(id),
    metadata TEXT,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
);

CREATE INDEX IF NOT EXISTS idx_payment_refunds_order ON payment_refunds(payment_order_id);
CREATE INDEX IF NOT EXISTS idx_payment_refunds_order_id ON payment_refunds(order_id);
