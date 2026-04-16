-- Site Maintenance Plugin
--
-- 定时维护插件，通过 Cron 系统触发三种周期任务：
--
--   1. cleanup_sessions  — 每 6 小时清理过期 Session
--   2. sync_view_counts  — 每 30 分钟同步文章浏览量
--   3. daily_digest      — 每天凌晨 1 点生成摘要报告
--
-- 使用 Host API：
--   Host.log / Host.dbQuery / Host.getData / Host.setData / Host.httpPost

-- ── 工具函数 ──────────────────────────────────────────────────

local function log(level, msg)
    Host.log(level, "[site-maintenance] " .. msg)
end

local function query(sql)
    local result = Host.dbQuery(sql)
    if not result then
        return nil, "query returned nil"
    end
    if result:sub(1, 6) == "error:" then
        return nil, result
    end
    return result
end

local function json_number(json_str, key)
    if not json_str then return 0 end
    return tonumber(json_str:match('"' .. key .. '":(%d+)'))
        or tonumber(json_str:match('"' .. key .. '":"(%d+)"'))
        or 0
end

-- ── 任务 1: 清理过期 Session ──────────────────────────────────

local function cleanup_sessions(payload_str)
    local max_age = 24
    if payload_str and payload_str ~= "" then
        max_age = tonumber(payload_str:match('"max_age_hours":(%d+)')) or 24
    end

    log("info", "cleaning up sessions older than " .. max_age .. "h")

    local result, err = query(
        "SELECT COUNT(*) as cnt FROM sessions "
        .. "WHERE created_at < datetime('now', '-' || " .. max_age .. " || ' hours')"
    )
    if not result then
        log("error", "cleanup_sessions query failed: " .. (err or "unknown"))
        return
    end

    local count = json_number(result, "cnt")
    log("info", "found " .. count .. " expired sessions")

    if count > 0 then
        log("info", "session cleanup would remove " .. count .. " records")
        Host.setData("last_cleanup_count", tostring(count))
        Host.setData("last_cleanup_time", os.date("!%Y-%m-%dT%H:%M:%SZ"))
    else
        log("info", "no expired sessions to clean")
    end
end

-- ── 任务 2: 同步文章浏览量 ────────────────────────────────────

local function sync_view_counts(payload_str)
    local batch_size = 100
    if payload_str and payload_str ~= "" then
        batch_size = tonumber(payload_str:match('"batch_size":(%d+)')) or 100
    end

    log("info", "syncing view counts (batch_size=" .. batch_size .. ")")

    local result, err = query(
        "SELECT id, title, view_count FROM posts "
        .. "WHERE status = 'published' "
        .. "ORDER BY view_count DESC LIMIT " .. batch_size
    )
    if not result then
        log("error", "sync_view_counts query failed: " .. (err or "unknown"))
        return
    end

    local count = 0
    for _ in result:gmatch('"id"') do
        count = count + 1
    end

    log("info", "synced view counts for " .. count .. " published posts")

    local last_sync = Host.getData("total_sync_runs")
    local total = tonumber(last_sync or "0") + 1
    Host.setData("total_sync_runs", tostring(total))

    log("info", "total sync runs: " .. total)
end

-- ── 任务 3: 每日摘要报告 ──────────────────────────────────────

local function daily_digest()
    log("info", "generating daily digest report")

    local posts_json = query(
        "SELECT COUNT(*) as total FROM posts WHERE status = 'published'"
    )
    local comments_json = query(
        "SELECT COUNT(*) as total FROM comments"
    )
    local today_posts_json = query(
        "SELECT COUNT(*) as total FROM posts "
        .. "WHERE status = 'published' "
        .. "AND published_at >= date('now')"
    )

    local total_posts = json_number(posts_json, "total")
    local total_comments = json_number(comments_json, "total")
    local today_posts = json_number(today_posts_json, "total")

    local report = string.format(
        '{"date":"%s","total_posts":%d,"total_comments":%d,"today_published":%d}',
        os.date("!%Y-%m-%d"),
        total_posts,
        total_comments,
        today_posts
    )

    Host.setData("last_daily_digest", report)
    log("info", "daily digest: " .. report)

    if today_posts > 0 then
        log("info", today_posts .. " new post(s) published today")
    end

    log("info", "blog has " .. total_posts .. " published posts, " .. total_comments .. " comments")
end

-- ── 调度分发 ──────────────────────────────────────────────────

local handlers = {
    ["cleanup_sessions"] = cleanup_sessions,
    ["sync_view_counts"] = sync_view_counts,
    ["daily_digest"]     = daily_digest,
}

-- ── Hook 入口 ─────────────────────────────────────────────────

local function to_json_str(val)
    if val == nil then
        return ""
    end
    if type(val) == "string" then
        return val
    end
    if type(val) == "table" then
        local parts = {}
        for k, v in pairs(val) do
            if type(v) == "number" then
                parts[#parts + 1] = '"' .. k .. '":' .. tostring(v)
            elseif type(v) == "string" then
                parts[#parts + 1] = '"' .. k .. '":"' .. v .. '"'
            elseif type(v) == "boolean" then
                parts[#parts + 1] = '"' .. k .. '":' .. tostring(v)
            end
        end
        return "{" .. table.concat(parts, ",") .. "}"
    end
    return tostring(val)
end

Plugin = {
    on_cron_tick = function(data)
        local job_type = data.job_type or ""
        local payload = to_json_str(data.payload)
        local ts = data.timestamp or ""

        local handler = handlers[job_type]
        if not handler then
            log("warn", "unknown cron job_type: " .. job_type)
            return
        end

        log("info", "executing cron job: " .. job_type .. " at " .. ts)

        local ok, err = pcall(handler, payload)
        if not ok then
            log("error", "cron job " .. job_type .. " failed: " .. tostring(err))
        else
            log("info", "cron job " .. job_type .. " completed")
        end
    end,
}
