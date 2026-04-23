-- 页面表
CREATE TABLE IF NOT EXISTS pages (
    id               TEXT PRIMARY KEY,
    tenant_id        TEXT NOT NULL DEFAULT 'default',
    title            TEXT NOT NULL,
    slug             TEXT NOT NULL,
    content          TEXT,
    blocks           TEXT,
    meta_title       TEXT,
    meta_description TEXT,
    og_image         TEXT,
    template         TEXT NOT NULL DEFAULT 'default',
    parent_id        TEXT REFERENCES pages(id) ON DELETE SET NULL,
    sort_order       INTEGER NOT NULL DEFAULT 0,
    status           TEXT NOT NULL DEFAULT 'draft',
    author_id        TEXT NOT NULL REFERENCES users(id),
    cover_image      TEXT,
    published_at     TEXT,
    created_at       TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at       TEXT NOT NULL DEFAULT (datetime('now')),
    UNIQUE(tenant_id, slug)
);

CREATE INDEX idx_pages_slug      ON pages(tenant_id, slug);
CREATE INDEX idx_pages_status    ON pages(tenant_id, status);
CREATE INDEX idx_pages_parent    ON pages(tenant_id, parent_id);
CREATE INDEX idx_pages_author    ON pages(author_id);

-- 可复用块
CREATE TABLE IF NOT EXISTS reusable_blocks (
    id          TEXT PRIMARY KEY,
    tenant_id   TEXT NOT NULL DEFAULT 'default',
    name        TEXT NOT NULL,
    block_type  TEXT NOT NULL,
    content     TEXT NOT NULL,
    description TEXT,
    created_at  TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at  TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX idx_reusable_blocks_tenant ON reusable_blocks(tenant_id);
