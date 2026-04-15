-- 动态 RBAC：角色 + 权限表

CREATE TABLE IF NOT EXISTS roles (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL UNIQUE,
    description TEXT,
    is_system BOOLEAN NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS permissions (
    id TEXT PRIMARY KEY,
    role_id TEXT NOT NULL REFERENCES roles(id) ON DELETE CASCADE,
    action TEXT NOT NULL,
    subject TEXT NOT NULL,
    fields TEXT,
    conditions TEXT,
    created_at TEXT NOT NULL
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_permissions_role_action_subject
    ON permissions(role_id, action, subject);

-- 预置系统角色
INSERT OR IGNORE INTO roles (id, name, description, is_system, created_at, updated_at) VALUES
    ('role-admin', 'admin', '超级管理员', 1, datetime('now'), datetime('now')),
    ('role-editor', 'editor', '编辑', 0, datetime('now'), datetime('now')),
    ('role-author', 'author', '作者', 0, datetime('now'), datetime('now')),
    ('role-reader', 'reader', '读者', 1, datetime('now'), datetime('now'));

-- admin 角色拥有所有权限（通配符）
INSERT OR IGNORE INTO permissions (id, role_id, action, subject, fields, conditions, created_at) VALUES
    ('perm-admin-all', 'role-admin', '*', '*', '["*"]', NULL, datetime('now'));

-- editor 权限
INSERT OR IGNORE INTO permissions (id, role_id, action, subject, fields, conditions, created_at) VALUES
    ('perm-editor-ct-create', 'role-editor', 'content-type::*.*', 'content-type::*', '["*"]', NULL, datetime('now'));

-- author 权限（只能操作自己的内容）
INSERT OR IGNORE INTO permissions (id, role_id, action, subject, fields, conditions, created_at) VALUES
    ('perm-author-post-create', 'role-author', 'content-type::post.create', 'content-type::post', '["*"]', NULL, datetime('now')),
    ('perm-author-post-read', 'role-author', 'content-type::post.read', 'content-type::post', '["*"]', NULL, datetime('now')),
    ('perm-author-post-update', 'role-author', 'content-type::post.update', 'content-type::post', '["*"]', '{"author_id":"$user.id"}', datetime('now')),
    ('perm-author-post-delete', 'role-author', 'content-type::post.delete', 'content-type::post', '["*"]', '{"author_id":"$user.id"}', datetime('now'));

-- reader 权限
INSERT OR IGNORE INTO permissions (id, role_id, action, subject, fields, conditions, created_at) VALUES
    ('perm-reader-post-read', 'role-reader', 'content-type::post.read', 'content-type::post', '["title","slug","content","excerpt","status"]', NULL, datetime('now')),
    ('perm-reader-comment-create', 'role-reader', 'content-type::comment.create', 'content-type::comment', '["content","nickname","email"]', NULL, datetime('now'));
