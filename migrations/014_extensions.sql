CREATE TABLE IF NOT EXISTS extensions (
    id           TEXT PRIMARY KEY,
    name         TEXT NOT NULL,
    version      TEXT NOT NULL,
    enabled      INTEGER NOT NULL DEFAULT 1,
    config       TEXT,
    installed_at TEXT NOT NULL,
    updated_at   TEXT NOT NULL,
    tenant_id    TEXT
);
