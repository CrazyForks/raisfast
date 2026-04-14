-- 后台任务队列表
CREATE TABLE IF NOT EXISTS jobs (
    id           TEXT PRIMARY KEY,
    job_type     TEXT NOT NULL,
    payload      TEXT NOT NULL,
    status       TEXT NOT NULL DEFAULT 'pending',
    attempts     INTEGER NOT NULL DEFAULT 0,
    max_attempts INTEGER NOT NULL DEFAULT 3,
    run_after    TEXT,
    error        TEXT,
    created_at   TEXT NOT NULL,
    updated_at   TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_jobs_status ON jobs(status);
CREATE INDEX IF NOT EXISTS idx_jobs_run_after ON jobs(run_after) WHERE status = 'pending';
CREATE INDEX IF NOT EXISTS idx_jobs_type ON jobs(job_type);
