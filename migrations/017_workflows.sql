-- 工作流定义
CREATE TABLE IF NOT EXISTS workflow_definitions (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    description TEXT,
    steps TEXT NOT NULL,           -- JSON: [{id, name, type, config, next, timeout_ms}]
    initial_step TEXT NOT NULL,    -- 第一步的 step id
    version INTEGER NOT NULL DEFAULT 1,
    enabled INTEGER NOT NULL DEFAULT 1,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now'))
);

-- 工作流实例
CREATE TABLE IF NOT EXISTS workflow_instances (
    id TEXT PRIMARY KEY,
    definition_id TEXT NOT NULL REFERENCES workflow_definitions(id),
    status TEXT NOT NULL DEFAULT 'running',  -- running|paused|completed|failed|cancelled
    current_step TEXT,                         -- 当前步骤 id
    context TEXT NOT NULL DEFAULT '{}',        -- JSON: 工作流上下文数据
    triggered_by TEXT,                         -- 触发来源（user_id / event / cron）
    started_at TEXT NOT NULL DEFAULT (datetime('now')),
    completed_at TEXT,
    updated_at TEXT NOT NULL DEFAULT (datetime('now'))
);
CREATE INDEX IF NOT EXISTS idx_wf_instances_definition ON workflow_instances(definition_id);
CREATE INDEX IF NOT EXISTS idx_wf_instances_status ON workflow_instances(status);

-- 步骤执行日志
CREATE TABLE IF NOT EXISTS workflow_step_logs (
    id TEXT PRIMARY KEY,
    instance_id TEXT NOT NULL REFERENCES workflow_instances(id),
    step_id TEXT NOT NULL,
    step_name TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'running',   -- running|completed|failed|skipped
    input TEXT,                                 -- JSON
    output TEXT,                                -- JSON
    error TEXT,
    started_at TEXT NOT NULL DEFAULT (datetime('now')),
    completed_at TEXT
);
CREATE INDEX IF NOT EXISTS idx_wf_step_logs_instance ON workflow_step_logs(instance_id);
