-- 定时任务调度表
CREATE TABLE IF NOT EXISTS cron_schedules (
    id           TEXT PRIMARY KEY,
    label        TEXT NOT NULL,
    job_type     TEXT NOT NULL,
    payload      TEXT,
    cron_expr    TEXT NOT NULL,
    enabled      INTEGER NOT NULL DEFAULT 1,
    last_run_at  TEXT,
    next_run_at  TEXT NOT NULL,
    plugin_id    TEXT,
    created_at   TEXT NOT NULL,
    updated_at   TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_cron_enabled ON cron_schedules(enabled);
CREATE INDEX IF NOT EXISTS idx_cron_next_run ON cron_schedules(next_run_at) WHERE enabled = 1;
CREATE INDEX IF NOT EXISTS idx_cron_plugin ON cron_schedules(plugin_id);
