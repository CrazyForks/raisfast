-- ============================================================
-- raisfast 完整数据库 Schema — MySQL（BUILTIN_TENANTABLE=false 默认模式）
-- 由所有 migration 文件合并而成，用于新部署一键初始化
-- 生成日期：2026-05-07
--
-- 注意：此 schema 不含 tenant_id 列。
-- 若需多租户支持，设置 BUILTIN_TENANTABLE=true 后迁移 026 会自动添加。
--
-- MySQL 注意事项：
-- - 不支持 WHERE 条件的部分索引，已移除
-- - BOOLEAN 实际为 TINYINT(1)
-- ============================================================

-- ── 平台基础层（永不禁用） ──────────────────────────────────

-- 用户
CREATE TABLE IF NOT EXISTS users (
    id BIGINT AUTO_INCREMENT PRIMARY KEY,
    document_id VARCHAR(36) NOT NULL UNIQUE,
    email VARCHAR(255) UNIQUE NOT NULL,
    username VARCHAR(255) UNIQUE NOT NULL,
    password_hash VARCHAR(255) NOT NULL,
    role VARCHAR(50) NOT NULL DEFAULT 'reader',
    avatar VARCHAR(500),
    bio TEXT,
    website VARCHAR(500),
    phone VARCHAR(50),
    email_verified BOOLEAN NOT NULL DEFAULT FALSE,
    display_name VARCHAR(100),
    slug VARCHAR(100) UNIQUE,
    locale VARCHAR(10),
    created_at DATETIME(3) NOT NULL DEFAULT CURRENT_TIMESTAMP(3),
    updated_at DATETIME(3) NOT NULL DEFAULT CURRENT_TIMESTAMP(3)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;

CREATE INDEX idx_users_email ON users(email);
CREATE INDEX idx_users_username ON users(username);
CREATE UNIQUE INDEX idx_users_phone ON users(phone);

-- Refresh Tokens
CREATE TABLE IF NOT EXISTS refresh_tokens (
    id BIGINT AUTO_INCREMENT PRIMARY KEY,
    document_id VARCHAR(36) NOT NULL UNIQUE,
    user_id BIGINT NOT NULL,
    token VARCHAR(500) UNIQUE NOT NULL,
    expires_at DATETIME(3) NOT NULL,
    created_at DATETIME(3) NOT NULL DEFAULT CURRENT_TIMESTAMP(3),
    FOREIGN KEY (user_id) REFERENCES users(id)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;

CREATE INDEX idx_refresh_tokens_user ON refresh_tokens(user_id);
CREATE INDEX idx_refresh_tokens_token ON refresh_tokens(token);
CREATE INDEX idx_refresh_tokens_expires_at ON refresh_tokens(expires_at);

-- 站点配置
CREATE TABLE IF NOT EXISTS options (
    id BIGINT AUTO_INCREMENT PRIMARY KEY,
    document_id VARCHAR(36) NOT NULL UNIQUE,
    `option_key` VARCHAR(255) NOT NULL,
    value TEXT NOT NULL,
    `type` VARCHAR(50) NOT NULL DEFAULT 'text',
    group_name VARCHAR(100) NOT NULL DEFAULT 'general',
    label VARCHAR(255) NOT NULL DEFAULT '',
    description TEXT,
    validation JSON,
    is_public BOOLEAN NOT NULL DEFAULT FALSE,
    autoload BOOLEAN NOT NULL DEFAULT TRUE,
    sort_order INT NOT NULL DEFAULT 0,
    updated_at DATETIME(3) NOT NULL DEFAULT CURRENT_TIMESTAMP(3),
    UNIQUE KEY uq_options_option_key (`option_key`)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;

-- RBAC 角色
CREATE TABLE IF NOT EXISTS roles (
    id BIGINT AUTO_INCREMENT PRIMARY KEY,
    document_id VARCHAR(36) NOT NULL UNIQUE,
    name VARCHAR(100) NOT NULL UNIQUE,
    description TEXT,
    is_system BOOLEAN NOT NULL DEFAULT FALSE,
    created_at DATETIME(3) NOT NULL DEFAULT CURRENT_TIMESTAMP(3),
    updated_at DATETIME(3) NOT NULL DEFAULT CURRENT_TIMESTAMP(3)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;

-- RBAC 权限
CREATE TABLE IF NOT EXISTS permissions (
    id BIGINT AUTO_INCREMENT PRIMARY KEY,
    document_id VARCHAR(36) NOT NULL UNIQUE,
    role_id BIGINT NOT NULL,
    action VARCHAR(255) NOT NULL,
    subject VARCHAR(255) NOT NULL,
    fields JSON,
    conditions JSON,
    created_at DATETIME(3) NOT NULL DEFAULT CURRENT_TIMESTAMP(3),
    FOREIGN KEY (role_id) REFERENCES roles(id)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;

CREATE UNIQUE INDEX idx_permissions_role_action_subject
    ON permissions(role_id, action, subject);

-- 租户
CREATE TABLE IF NOT EXISTS tenants (
    id BIGINT AUTO_INCREMENT PRIMARY KEY,
    document_id VARCHAR(36) NOT NULL UNIQUE,
    name VARCHAR(255) NOT NULL,
    domain VARCHAR(255) UNIQUE,
    config JSON NOT NULL,
    status VARCHAR(50) NOT NULL DEFAULT 'active',
    created_at DATETIME(3) NOT NULL DEFAULT CURRENT_TIMESTAMP(3),
    updated_at DATETIME(3) NOT NULL DEFAULT CURRENT_TIMESTAMP(3)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;

-- 审计日志
CREATE TABLE IF NOT EXISTS audit_log (
    id BIGINT AUTO_INCREMENT PRIMARY KEY,
    document_id VARCHAR(36) NOT NULL UNIQUE,
    actor_id BIGINT,
    actor_role VARCHAR(50),
    action VARCHAR(255) NOT NULL,
    subject VARCHAR(255) NOT NULL,
    subject_id VARCHAR(36),
    detail TEXT,
    ip_address VARCHAR(45),
    user_agent TEXT,
    created_at DATETIME(3) NOT NULL DEFAULT CURRENT_TIMESTAMP(3)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;

CREATE INDEX idx_audit_log_action ON audit_log(action);
CREATE INDEX idx_audit_log_actor ON audit_log(actor_id);
CREATE INDEX idx_audit_log_created ON audit_log(created_at);

-- API Token
CREATE TABLE IF NOT EXISTS api_tokens (
    id BIGINT AUTO_INCREMENT PRIMARY KEY,
    document_id VARCHAR(36) NOT NULL UNIQUE,
    user_id BIGINT NOT NULL,
    name VARCHAR(255) NOT NULL,
    token_hash VARCHAR(255) UNIQUE NOT NULL,
    token_prefix VARCHAR(50) NOT NULL,
    scopes JSON NOT NULL,
    last_used_at DATETIME(3),
    expires_at DATETIME(3),
    created_at DATETIME(3) NOT NULL DEFAULT CURRENT_TIMESTAMP(3),
    FOREIGN KEY (user_id) REFERENCES users(id)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;

CREATE INDEX idx_api_tokens_user_id ON api_tokens(user_id);
CREATE INDEX idx_api_tokens_token_hash ON api_tokens(token_hash);

-- Webhook 订阅
CREATE TABLE IF NOT EXISTS webhook_subscriptions (
    id BIGINT AUTO_INCREMENT PRIMARY KEY,
    document_id VARCHAR(36) NOT NULL UNIQUE,
    url VARCHAR(1024) NOT NULL,
    secret VARCHAR(255) NOT NULL,
    events JSON NOT NULL,
    enabled BOOLEAN NOT NULL DEFAULT TRUE,
    description TEXT,
    created_at DATETIME(3) NOT NULL DEFAULT CURRENT_TIMESTAMP(3),
    updated_at DATETIME(3) NOT NULL DEFAULT CURRENT_TIMESTAMP(3)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;

CREATE INDEX idx_webhook_subscriptions_enabled ON webhook_subscriptions(enabled);

-- 插件 KV 存储
CREATE TABLE IF NOT EXISTS plugin_storage (
    plugin_id VARCHAR(100) NOT NULL,
    `storage_key` VARCHAR(255) NOT NULL,
    value TEXT NOT NULL,
    expires_at DATETIME(3),
    updated_at DATETIME(3) NOT NULL DEFAULT CURRENT_TIMESTAMP(3),
    PRIMARY KEY (plugin_id, `storage_key`)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;

CREATE INDEX idx_plugin_storage_plugin ON plugin_storage(plugin_id);

-- 内容版本历史
CREATE TABLE IF NOT EXISTS content_revisions (
    id BIGINT AUTO_INCREMENT PRIMARY KEY,
    document_id VARCHAR(36) NOT NULL UNIQUE,
    content_type VARCHAR(100) NOT NULL,
    record_id TEXT NOT NULL,
    revision_number INT NOT NULL,
    snapshot TEXT NOT NULL,
    created_by BIGINT,
    created_at DATETIME(3) NOT NULL DEFAULT CURRENT_TIMESTAMP(3),
    UNIQUE KEY uq_revision (content_type, record_id, revision_number)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;

CREATE INDEX idx_revisions_ct_record
    ON content_revisions(content_type, record_id);
CREATE INDEX idx_revisions_ct_record_rev
    ON content_revisions(content_type, record_id, revision_number DESC);

-- OAuth 账号绑定
CREATE TABLE IF NOT EXISTS oauth_accounts (
    id BIGINT AUTO_INCREMENT PRIMARY KEY,
    document_id VARCHAR(36) NOT NULL UNIQUE,
    user_id BIGINT NOT NULL,
    provider VARCHAR(50) NOT NULL,
    provider_user_id VARCHAR(255) NOT NULL,
    email VARCHAR(255),
    display_name VARCHAR(255),
    avatar_url VARCHAR(500),
    access_token VARCHAR(1024),
    refresh_token VARCHAR(1024),
    token_expires_at DATETIME(3),
    profile TEXT,
    created_at DATETIME(3) NOT NULL DEFAULT CURRENT_TIMESTAMP(3),
    updated_at DATETIME(3) NOT NULL DEFAULT CURRENT_TIMESTAMP(3),
    UNIQUE KEY uq_oauth_provider (provider, provider_user_id),
    FOREIGN KEY (user_id) REFERENCES users(id)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;

CREATE INDEX idx_oauth_accounts_user ON oauth_accounts(user_id);
CREATE INDEX idx_oauth_accounts_provider ON oauth_accounts(provider, provider_user_id);

-- OAuth 短期 state 存储（PKCE）
CREATE TABLE IF NOT EXISTS oauth_states (
    id BIGINT AUTO_INCREMENT PRIMARY KEY,
    document_id VARCHAR(36) NOT NULL UNIQUE,
    provider VARCHAR(50) NOT NULL,
    code_verifier VARCHAR(255) NOT NULL,
    user_id BIGINT,
    created_at DATETIME(3) NOT NULL DEFAULT CURRENT_TIMESTAMP(3),
    expires_at DATETIME(3) NOT NULL
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;

CREATE INDEX idx_oauth_states_expires ON oauth_states(expires_at);

-- 密码重置令牌
CREATE TABLE IF NOT EXISTS password_reset_tokens (
    id BIGINT AUTO_INCREMENT PRIMARY KEY,
    document_id VARCHAR(36) NOT NULL UNIQUE,
    user_id BIGINT NOT NULL,
    token VARCHAR(255) NOT NULL UNIQUE,
    expires_at DATETIME(3) NOT NULL,
    used_at DATETIME(3),
    created_at DATETIME(3) NOT NULL DEFAULT CURRENT_TIMESTAMP(3),
    FOREIGN KEY (user_id) REFERENCES users(id)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;

CREATE INDEX idx_password_reset_tokens_token ON password_reset_tokens(token);
CREATE INDEX idx_password_reset_tokens_user_id ON password_reset_tokens(user_id);
CREATE INDEX idx_password_reset_tokens_expires_at ON password_reset_tokens(expires_at);

-- 短信验证码
CREATE TABLE IF NOT EXISTS sms_codes (
    id BIGINT AUTO_INCREMENT PRIMARY KEY,
    document_id VARCHAR(36) NOT NULL UNIQUE,
    phone VARCHAR(50) NOT NULL,
    code VARCHAR(20) NOT NULL,
    purpose VARCHAR(50) NOT NULL,
    expires_at DATETIME(3) NOT NULL,
    verified_at DATETIME(3),
    attempts INT NOT NULL DEFAULT 0,
    ip_address VARCHAR(45),
    created_at DATETIME(3) NOT NULL DEFAULT CURRENT_TIMESTAMP(3)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;

CREATE INDEX idx_sms_codes_phone ON sms_codes(phone);
CREATE INDEX idx_sms_codes_expires ON sms_codes(expires_at);

-- 邮箱验证令牌
CREATE TABLE IF NOT EXISTS email_verification_tokens (
    id BIGINT AUTO_INCREMENT PRIMARY KEY,
    document_id VARCHAR(36) NOT NULL UNIQUE,
    user_id BIGINT NOT NULL,
    token VARCHAR(255) NOT NULL UNIQUE,
    email VARCHAR(255) NOT NULL,
    expires_at DATETIME(3) NOT NULL,
    verified_at DATETIME(3),
    created_at DATETIME(3) NOT NULL DEFAULT CURRENT_TIMESTAMP(3),
    FOREIGN KEY (user_id) REFERENCES users(id)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;

CREATE INDEX idx_email_verification_tokens_token ON email_verification_tokens(token);
CREATE INDEX idx_email_verification_tokens_user_id ON email_verification_tokens(user_id);
CREATE INDEX idx_email_verification_tokens_expires ON email_verification_tokens(expires_at);

-- 后台任务队列
CREATE TABLE IF NOT EXISTS jobs (
    id           BIGINT AUTO_INCREMENT PRIMARY KEY,
    document_id  VARCHAR(36) NOT NULL UNIQUE,
    job_type     VARCHAR(100) NOT NULL,
    payload      TEXT NOT NULL,
    status       VARCHAR(50) NOT NULL DEFAULT 'pending',
    attempts     INT NOT NULL DEFAULT 0,
    max_attempts INT NOT NULL DEFAULT 3,
    run_after    DATETIME(3),
    error        TEXT,
    created_at   DATETIME(3) NOT NULL DEFAULT CURRENT_TIMESTAMP(3),
    updated_at   DATETIME(3) NOT NULL DEFAULT CURRENT_TIMESTAMP(3)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;

CREATE INDEX idx_jobs_status ON jobs(status);
CREATE INDEX idx_jobs_run_after ON jobs(run_after);
CREATE INDEX idx_jobs_type ON jobs(job_type);

-- 定时任务调度
CREATE TABLE IF NOT EXISTS cron_schedules (
    id           BIGINT AUTO_INCREMENT PRIMARY KEY,
    document_id  VARCHAR(36) NOT NULL UNIQUE,
    label        VARCHAR(255) NOT NULL,
    job_type     VARCHAR(100) NOT NULL,
    payload      TEXT,
    cron_expr    VARCHAR(100) NOT NULL,
    enabled      BOOLEAN NOT NULL DEFAULT TRUE,
    last_run_at  DATETIME(3),
    next_run_at  DATETIME(3) NOT NULL,
    plugin_id    VARCHAR(100),
    created_at   DATETIME(3) NOT NULL DEFAULT CURRENT_TIMESTAMP(3),
    updated_at   DATETIME(3) NOT NULL DEFAULT CURRENT_TIMESTAMP(3)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;

CREATE INDEX idx_cron_enabled ON cron_schedules(enabled);
CREATE INDEX idx_cron_next_run ON cron_schedules(next_run_at);
CREATE INDEX idx_cron_plugin ON cron_schedules(plugin_id);

-- Cron 执行历史
CREATE TABLE IF NOT EXISTS cron_execution_log (
    id           BIGINT AUTO_INCREMENT PRIMARY KEY,
    document_id  VARCHAR(36) NOT NULL UNIQUE,
    schedule_id  BIGINT NOT NULL,
    job_type     VARCHAR(100) NOT NULL,
    label        VARCHAR(255) NOT NULL,
    status       VARCHAR(50) NOT NULL DEFAULT 'running',
    duration_ms  INT,
    error        TEXT,
    started_at   DATETIME(3) NOT NULL,
    finished_at  DATETIME(3)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;

CREATE INDEX idx_cron_log_schedule ON cron_execution_log(schedule_id);
CREATE INDEX idx_cron_log_status ON cron_execution_log(status);
CREATE INDEX idx_cron_log_started ON cron_execution_log(started_at);

-- ── 内置模块：Blog（BUILTIN_BLOG=true） ──────────────────

-- 分类
CREATE TABLE IF NOT EXISTS categories (
    id BIGINT AUTO_INCREMENT PRIMARY KEY,
    document_id VARCHAR(36) NOT NULL UNIQUE,
    name VARCHAR(255) UNIQUE NOT NULL,
    slug VARCHAR(255) UNIQUE NOT NULL,
    description TEXT,
    parent_id BIGINT,
    sort_order INT NOT NULL DEFAULT 0,
    created_by BIGINT,
    updated_by BIGINT,
    cover_image VARCHAR(500),
    meta_title VARCHAR(255),
    meta_description VARCHAR(500),
    og_title VARCHAR(255),
    og_description VARCHAR(500),
    og_image VARCHAR(500),
    created_at DATETIME(3) NOT NULL DEFAULT CURRENT_TIMESTAMP(3),
    updated_at DATETIME(3) NOT NULL DEFAULT CURRENT_TIMESTAMP(3),
    FOREIGN KEY (parent_id) REFERENCES categories(id)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;

-- 标签
CREATE TABLE IF NOT EXISTS tags (
    id BIGINT AUTO_INCREMENT PRIMARY KEY,
    document_id VARCHAR(36) NOT NULL UNIQUE,
    name VARCHAR(255) UNIQUE NOT NULL,
    slug VARCHAR(255) UNIQUE NOT NULL,
    created_by BIGINT,
    updated_by BIGINT,
    description TEXT,
    cover_image VARCHAR(500),
    meta_title VARCHAR(255),
    meta_description VARCHAR(500),
    og_title VARCHAR(255),
    og_description VARCHAR(500),
    og_image VARCHAR(500),
    created_at DATETIME(3) NOT NULL DEFAULT CURRENT_TIMESTAMP(3),
    updated_at DATETIME(3) NOT NULL DEFAULT CURRENT_TIMESTAMP(3)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;

-- 文章
CREATE TABLE IF NOT EXISTS posts (
    id BIGINT AUTO_INCREMENT PRIMARY KEY,
    document_id VARCHAR(36) NOT NULL UNIQUE,
    title VARCHAR(500) NOT NULL,
    slug VARCHAR(255) UNIQUE NOT NULL,
    content LONGTEXT NOT NULL,
    excerpt TEXT,
    cover_image VARCHAR(500),
    status VARCHAR(50) NOT NULL DEFAULT 'draft',
    created_by BIGINT NOT NULL,
    updated_by BIGINT,
    category_id BIGINT,
    view_count INT NOT NULL DEFAULT 0,
    is_pinned BOOLEAN NOT NULL DEFAULT FALSE,
    password VARCHAR(255),
    comment_status VARCHAR(20) NOT NULL DEFAULT 'open',
    format VARCHAR(20) NOT NULL DEFAULT 'standard',
    template VARCHAR(100) NOT NULL DEFAULT 'default',
    meta_title VARCHAR(255),
    meta_description VARCHAR(500),
    og_title VARCHAR(255),
    og_description VARCHAR(500),
    og_image VARCHAR(500),
    canonical_url VARCHAR(1024),
    reading_time INTEGER NOT NULL DEFAULT 0,
    created_at DATETIME(3) NOT NULL DEFAULT CURRENT_TIMESTAMP(3),
    updated_at DATETIME(3) NOT NULL DEFAULT CURRENT_TIMESTAMP(3),
    published_at DATETIME(3),
    FOREIGN KEY (created_by) REFERENCES users(id),
    FOREIGN KEY (category_id) REFERENCES categories(id)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;

CREATE INDEX idx_posts_slug ON posts(slug);
CREATE INDEX idx_posts_status ON posts(status);
CREATE INDEX idx_posts_author ON posts(created_by);
CREATE INDEX idx_posts_category ON posts(category_id);
CREATE INDEX idx_posts_created ON posts(created_at);
CREATE INDEX idx_posts_status_created
    ON posts(status, is_pinned DESC, created_at DESC);
CREATE INDEX idx_posts_status_category
    ON posts(status, category_id);
CREATE INDEX idx_posts_status_author
    ON posts(status, created_by);

-- 文章-标签（多对多）
CREATE TABLE IF NOT EXISTS posts_tags (
    post_id BIGINT NOT NULL,
    tag_id BIGINT NOT NULL,
    PRIMARY KEY (post_id, tag_id),
    FOREIGN KEY (post_id) REFERENCES posts(id),
    FOREIGN KEY (tag_id) REFERENCES tags(id)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;

CREATE INDEX idx_posts_tags_tag_id ON posts_tags(tag_id);

-- 评论
CREATE TABLE IF NOT EXISTS comments (
    id BIGINT AUTO_INCREMENT PRIMARY KEY,
    document_id VARCHAR(36) NOT NULL UNIQUE,
    post_id BIGINT NOT NULL,
    created_by BIGINT,
    updated_by BIGINT,
    nickname VARCHAR(100),
    email VARCHAR(255),
    content TEXT NOT NULL,
    parent_id BIGINT,
    status VARCHAR(50) NOT NULL DEFAULT 'pending',
    author_ip VARCHAR(45),
    author_url VARCHAR(500),
    created_at DATETIME(3) NOT NULL DEFAULT CURRENT_TIMESTAMP(3),
    updated_at DATETIME(3) NOT NULL DEFAULT CURRENT_TIMESTAMP(3),
    FOREIGN KEY (post_id) REFERENCES posts(id),
    FOREIGN KEY (created_by) REFERENCES users(id),
    FOREIGN KEY (parent_id) REFERENCES comments(id)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;

CREATE INDEX idx_comments_post ON comments(post_id);
CREATE INDEX idx_comments_status ON comments(status);
CREATE INDEX idx_comments_post_status
    ON comments(post_id, status);
CREATE INDEX idx_comments_parent_id
    ON comments(parent_id);

-- ── 内置模块：Pages（BUILTIN_PAGES=true） ────────────────

CREATE TABLE IF NOT EXISTS pages (
    id               BIGINT AUTO_INCREMENT PRIMARY KEY,
    document_id      VARCHAR(36) NOT NULL UNIQUE,
    title            VARCHAR(500) NOT NULL,
    slug             VARCHAR(255) NOT NULL UNIQUE,
    content          LONGTEXT,
    blocks           JSON,
    meta_title       VARCHAR(255),
    meta_description VARCHAR(500),
    og_image         VARCHAR(500),
    template         VARCHAR(100) NOT NULL DEFAULT 'default',
    parent_id        BIGINT,
    sort_order       INT NOT NULL DEFAULT 0,
    status           VARCHAR(50) NOT NULL DEFAULT 'draft',
    created_by       BIGINT NOT NULL,
    updated_by       BIGINT,
    cover_image      VARCHAR(500),
    published_at     DATETIME(3),
    password         VARCHAR(255),
    comment_status   VARCHAR(20) NOT NULL DEFAULT 'closed',
    og_title         VARCHAR(255),
    og_description   VARCHAR(500),
    canonical_url    VARCHAR(1024),
    created_at       DATETIME(3) NOT NULL DEFAULT CURRENT_TIMESTAMP(3),
    updated_at       DATETIME(3) NOT NULL DEFAULT CURRENT_TIMESTAMP(3),
    FOREIGN KEY (created_by) REFERENCES users(id),
    FOREIGN KEY (parent_id) REFERENCES pages(id)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;

CREATE INDEX idx_pages_slug      ON pages(slug);
CREATE INDEX idx_pages_status    ON pages(status);
CREATE INDEX idx_pages_parent    ON pages(parent_id);
CREATE INDEX idx_pages_author    ON pages(created_by);

CREATE TABLE IF NOT EXISTS reusable_blocks (
    id          BIGINT AUTO_INCREMENT PRIMARY KEY,
    document_id VARCHAR(36) NOT NULL UNIQUE,
    name        VARCHAR(255) NOT NULL,
    block_type  VARCHAR(100) NOT NULL,
    content     LONGTEXT NOT NULL,
    description TEXT,
    created_by  BIGINT,
    updated_by  BIGINT,
    created_at  DATETIME(3) NOT NULL DEFAULT CURRENT_TIMESTAMP(3),
    updated_at  DATETIME(3) NOT NULL DEFAULT CURRENT_TIMESTAMP(3)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;

-- ── 内置模块：Media（BUILTIN_MEDIA=true） ────────────────

CREATE TABLE IF NOT EXISTS media (
    id BIGINT AUTO_INCREMENT PRIMARY KEY,
    document_id VARCHAR(36) NOT NULL UNIQUE,
    user_id BIGINT NOT NULL,
    filename VARCHAR(255) NOT NULL,
    filepath VARCHAR(500) NOT NULL,
    mimetype VARCHAR(100) NOT NULL,
    size BIGINT NOT NULL,
    width INT,
    height INT,
    title VARCHAR(255),
    alt_text VARCHAR(255),
    caption TEXT,
    description TEXT,
    updated_at DATETIME(3) NOT NULL DEFAULT CURRENT_TIMESTAMP(3),
    created_at DATETIME(3) NOT NULL DEFAULT CURRENT_TIMESTAMP(3),
    FOREIGN KEY (user_id) REFERENCES users(id)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;

CREATE INDEX idx_media_user_created
    ON media(user_id, created_at DESC);

-- ── 内置模块：Workflow（BUILTIN_WORKFLOW=true） ──────────

CREATE TABLE IF NOT EXISTS workflow_definitions (
    id BIGINT AUTO_INCREMENT PRIMARY KEY,
    document_id VARCHAR(36) NOT NULL UNIQUE,
    name VARCHAR(255) NOT NULL,
    description TEXT,
    steps JSON NOT NULL,
    initial_step VARCHAR(100) NOT NULL,
    version INT NOT NULL DEFAULT 1,
    enabled BOOLEAN NOT NULL DEFAULT TRUE,
    created_at DATETIME(3) NOT NULL DEFAULT CURRENT_TIMESTAMP(3),
    updated_at DATETIME(3) NOT NULL DEFAULT CURRENT_TIMESTAMP(3)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;

CREATE TABLE IF NOT EXISTS workflow_instances (
    id BIGINT AUTO_INCREMENT PRIMARY KEY,
    document_id VARCHAR(36) NOT NULL UNIQUE,
    definition_id BIGINT NOT NULL,
    status VARCHAR(50) NOT NULL DEFAULT 'running',
    current_step VARCHAR(100),
    context JSON NOT NULL,
    triggered_by BIGINT,
    started_at DATETIME(3) NOT NULL DEFAULT CURRENT_TIMESTAMP(3),
    completed_at DATETIME(3),
    updated_at DATETIME(3) NOT NULL DEFAULT CURRENT_TIMESTAMP(3),
    FOREIGN KEY (definition_id) REFERENCES workflow_definitions(id)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;

CREATE INDEX idx_wf_instances_definition ON workflow_instances(definition_id);
CREATE INDEX idx_wf_instances_status ON workflow_instances(status);

CREATE TABLE IF NOT EXISTS workflow_step_logs (
    id BIGINT AUTO_INCREMENT PRIMARY KEY,
    document_id VARCHAR(36) NOT NULL UNIQUE,
    instance_id BIGINT NOT NULL,
    step_id VARCHAR(100) NOT NULL,
    step_name VARCHAR(255) NOT NULL,
    status VARCHAR(50) NOT NULL DEFAULT 'running',
    input LONGTEXT,
    output LONGTEXT,
    error TEXT,
    started_at DATETIME(3) NOT NULL DEFAULT CURRENT_TIMESTAMP(3),
    completed_at DATETIME(3),
    FOREIGN KEY (instance_id) REFERENCES workflow_instances(id)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;

CREATE INDEX idx_wf_step_logs_instance ON workflow_step_logs(instance_id);

-- ============================================================
-- 预置数据
-- ============================================================

-- 默认租户
INSERT IGNORE INTO tenants (document_id, name, domain, config, status, created_at, updated_at) VALUES
    ('default', 'Default', NULL, '{}', 'active', NOW(), NOW());

-- 系统角色
INSERT IGNORE INTO roles (document_id, name, description, is_system, created_at, updated_at) VALUES
    ('role-admin', 'admin', '超级管理员', TRUE, NOW(), NOW()),
    ('role-editor', 'editor', '编辑', FALSE, NOW(), NOW()),
    ('role-author', 'author', '作者', FALSE, NOW(), NOW()),
    ('role-reader', 'reader', '读者', TRUE, NOW(), NOW());

-- admin 全局权限
INSERT IGNORE INTO permissions (document_id, role_id, action, subject, fields, conditions, created_at) VALUES
    ('perm-admin-all', (SELECT id FROM roles WHERE document_id = 'role-admin'), '*', '*', '["*"]', NULL, NOW());

-- editor 权限
INSERT IGNORE INTO permissions (document_id, role_id, action, subject, fields, conditions, created_at) VALUES
    ('perm-editor-ct-create', (SELECT id FROM roles WHERE document_id = 'role-editor'), 'content-type::*.*', 'content-type::*', '["*"]', NULL, NOW());

-- author 权限
INSERT IGNORE INTO permissions (document_id, role_id, action, subject, fields, conditions, created_at) VALUES
    ('perm-author-post-create', (SELECT id FROM roles WHERE document_id = 'role-author'), 'content-type::post.create', 'content-type::post', '["*"]', NULL, NOW()),
    ('perm-author-post-read', (SELECT id FROM roles WHERE document_id = 'role-author'), 'content-type::post.read', 'content-type::post', '["*"]', NULL, NOW()),
    ('perm-author-post-update', (SELECT id FROM roles WHERE document_id = 'role-author'), 'content-type::post.update', 'content-type::post', '["*"]', '{"author_id":"$user.id"}', NOW()),
    ('perm-author-post-delete', (SELECT id FROM roles WHERE document_id = 'role-author'), 'content-type::post.delete', 'content-type::post', '["*"]', '{"author_id":"$user.id"}', NOW());

-- reader 权限
INSERT IGNORE INTO permissions (document_id, role_id, action, subject, fields, conditions, created_at) VALUES
    ('perm-reader-post-read', (SELECT id FROM roles WHERE document_id = 'role-reader'), 'content-type::post.read', 'content-type::post', '["title","slug","content","excerpt","status"]', NULL, NOW()),
    ('perm-reader-comment-create', (SELECT id FROM roles WHERE document_id = 'role-reader'), 'content-type::comment.create', 'content-type::comment', '["content","nickname","email"]', NULL, NOW());

-- 站点配置
INSERT IGNORE INTO options (document_id, `option_key`, value, `type`, group_name, label, description, validation, is_public, autoload, sort_order, updated_at) VALUES
    ('opt-site-title', 'site_title', '"My Blog"', 'text', 'general', '站点标题', '显示在浏览器标题栏和页面头部', '{"max_length":100}', TRUE, TRUE, 1, NOW()),
    ('opt-site-desc', 'site_description', '""', 'text', 'general', '站点描述', '简短描述站点用途', '{"max_length":500}', TRUE, TRUE, 2, NOW()),
    ('opt-site-url', 'site_url', '""', 'url', 'general', '站点 URL', '如 https://example.com', NULL, TRUE, TRUE, 3, NOW()),
    ('opt-admin-email', 'admin_email', '""', 'email', 'general', '管理员邮箱', NULL, NULL, FALSE, TRUE, 4, NOW()),
    ('opt-timezone', 'timezone', '"UTC"', 'select', 'general', '时区', NULL, '{"values":["UTC","Asia/Shanghai","Asia/Tokyo","US/Eastern","US/Pacific","Europe/London","Europe/Berlin"]}', TRUE, TRUE, 5, NOW()),
    ('opt-date-fmt', 'date_format', '"%Y-%m-%d"', 'select', 'general', '日期格式', NULL, '{"values":["%Y-%m-%d","%d/%m/%Y","%m/%d/%Y","%Y年%m月%d日"]}', TRUE, TRUE, 6, NOW()),
    ('opt-per-page', 'posts_per_page', '10', 'integer', 'reading', '每页文章数', NULL, '{"min":1,"max":100}', TRUE, TRUE, 10, NOW()),
    ('opt-rss-items', 'rss_items', '20', 'integer', 'reading', 'RSS 条目数', NULL, '{"min":1,"max":100}', TRUE, TRUE, 11, NOW()),
    ('opt-permalink', 'permalink_structure', '"/:year/:month/:slug"', 'select', 'reading', 'URL 结构', NULL, '{"values":["/:year/:month/:slug","/:slug","/posts/:slug"]}', TRUE, TRUE, 12, NOW()),
    ('opt-comment-mod', 'comment_moderation', 'true', 'boolean', 'discussion', '评论需审核', '开启后新评论需管理员审批', NULL, FALSE, TRUE, 20, NOW()),
    ('opt-comment-order', 'comment_order', '"asc"', 'select', 'discussion', '评论排序', NULL, '{"values":["asc","desc"]}', TRUE, TRUE, 21, NOW()),
    ('opt-default-role', 'default_role', '"reader"', 'select', 'discussion', '新用户默认角色', NULL, '{"values":["reader","author"]}', FALSE, TRUE, 22, NOW()),
    ('opt-theme', 'theme', '"default"', 'select', 'appearance', '当前主题', NULL, '{"values":["default","corporate","minimal","warm"]}', TRUE, TRUE, 30, NOW()),
    ('opt-maintenance', 'maintenance_mode', 'false', 'boolean', 'appearance', '维护模式', '开启后前台显示维护页面', NULL, TRUE, TRUE, 31, NOW());
