-- kb_query_logs observability columns (kb-observability-design §3.4).
-- Fresh installs get these via schema.sqlite.sql; existing DBs apply
-- this file via the `db migrate` CLI (tracked in `_migrations`).

ALTER TABLE kb_query_logs ADD COLUMN rewritten_question TEXT;
ALTER TABLE kb_query_logs ADD COLUMN kb_ids TEXT;
ALTER TABLE kb_query_logs ADD COLUMN latency_ms BIGINT;
ALTER TABLE kb_query_logs ADD COLUMN run_id BIGINT;
ALTER TABLE kb_query_logs ADD COLUMN error TEXT;
ALTER TABLE kb_query_logs ADD COLUMN source TEXT NOT NULL DEFAULT 'ask';
