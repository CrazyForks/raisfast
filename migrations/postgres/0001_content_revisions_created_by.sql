-- Baseline drift fix: content_revisions.created_by is written by the model
-- (models/content_revision.rs) but was absent from schema.postgres.sql, so any
-- PostgreSQL database created before the fix lacks the column.
-- Idempotent: databases already carrying the column (fresh installs from the
-- updated baseline) are unaffected.
ALTER TABLE content_revisions ADD COLUMN IF NOT EXISTS created_by BIGINT;
