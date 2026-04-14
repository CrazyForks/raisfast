-- Cron 执行历史日志表
CREATE TABLE IF NOT EXISTS cron_execution_log (
    id           TEXT PRIMARY KEY,
    schedule_id  TEXT NOT NULL,
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
