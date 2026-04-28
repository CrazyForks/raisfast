-- Blog Stats API Plugin
--
-- 通过 routes 声明式注册自定义 REST 端点，暴露博客统计数据。
-- 所有数据通过 dbQuery (SELECT) 从数据库读取。
--
-- 端点（在 manifest.toml [[routes]] 中声明）：
--   GET /api/v1/plugins/stats/overview   — 博客总览
--   GET /api/v1/plugins/stats/posts      — 文章统计
--   GET /api/v1/plugins/stats/recent     — 最近发布
--   GET /api/v1/plugins/stats/top        — 浏览量排行

-- ── 工具函数 ──────────────────────────────────────────────────

local function json_response(status, data)
    return {
        status = status,
        body = '{"code":0,"message":"ok","data":' .. data .. '}'
    }
end

local function error_response(status, msg)
    return {
        status = status,
        body = '{"code":' .. status .. '00,"message":"' .. msg .. '","data":null}'
    }
end

local function query(sql)
    local result = Host.dbQuery(sql)
    if not result then return nil, "query failed" end
    if result:sub(1, 6) == "error:" then return nil, result end
    return result
end

-- ── Route Handlers ──────────────────────────────────────────

Plugin = {}

Plugin.stats_overview = function(input)
    local posts_json, err = query("SELECT COUNT(*) as total FROM posts")
    if not posts_json then return error_response(500, err) end

    local comments_json, _ = query("SELECT COUNT(*) as total FROM comments")
    local users_json, _ = query("SELECT COUNT(*) as total FROM users")
    local published_json, _ = query("SELECT COUNT(*) as total FROM posts WHERE status = 'published'")

    local total_posts = tonumber(posts_json:match('"total":"?(%d+)"?')) or 0
    local total_comments = 0
    local total_users = 0
    local total_published = 0

    if comments_json then total_comments = tonumber(comments_json:match('"total":"?(%d+)"?')) or 0 end
    if users_json then total_users = tonumber(users_json:match('"total":"?(%d+)"?')) or 0 end
    if published_json then total_published = tonumber(published_json:match('"total":"?(%d+)"?')) or 0 end

    local data = '{"total_posts":' .. total_posts
        .. ',"total_published":' .. total_published
        .. ',"total_comments":' .. total_comments
        .. ',"total_users":' .. total_users .. '}'

    Host.log("info", "[blog-stats] GET overview")
    return json_response(200, data)
end

Plugin.stats_posts = function(input)
    local result, err = query("SELECT status, COUNT(*) as count FROM posts GROUP BY status")
    if not result then return error_response(500, err) end
    return json_response(200, result)
end

Plugin.stats_recent = function(input)
    local result, err = query(
        "SELECT title, slug, view_count, published_at "
        .. "FROM posts WHERE status = 'published' "
        .. "ORDER BY published_at DESC LIMIT 10"
    )
    if not result then return error_response(500, err) end
    return json_response(200, result)
end

Plugin.stats_top = function(input)
    local result, err = query(
        "SELECT title, slug, view_count "
        .. "FROM posts WHERE status = 'published' "
        .. "ORDER BY view_count DESC LIMIT 10"
    )
    if not result then return error_response(500, err) end
    return json_response(200, result)
end
