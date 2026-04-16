-- Blog Stats API Plugin
--
-- 通过 handle_route hook 注册自定义 REST 端点，暴露博客统计数据。
-- 所有数据通过 dbQuery (SELECT) 从数据库读取。
--
-- 端点：
--   GET /api/v1/plugins/stats/overview   — 博客总览（文章/评论/用户数量）
--   GET /api/v1/plugins/stats/posts      — 文章统计（按状态分组）
--   GET /api/v1/plugins/stats/recent     — 最近发布的文章列表
--   GET /api/v1/plugins/stats/top        — 浏览量排行

-- ── 工具函数 ──────────────────────────────────────────────────

local function json_response(status, data)
    return {
        status = status,
        body = '{' .. '"code":0,"message":"ok","data":' .. data .. '}'
    }
end

local function error_response(status, msg)
    return {
        status = status,
        body = '{"code":' .. status .. '00,"message":"' .. msg .. '","data":null}'
    }
end

-- 安全地从 dbQuery 结果中提取 JSON 数组字符串
-- dbQuery 返回格式: [{"col1":"val1","col2":"val2"}, ...]
local function query(sql)
    local result = Host.dbQuery(sql)
    if not result then
        return nil, "query failed"
    end
    -- dbQuery 错误时返回 "error: ..."
    if result:sub(1, 6) == "error:" then
        return nil, result
    end
    return result
end

-- ── 路由处理 ──────────────────────────────────────────────────

-- GET /api/v1/plugins/stats/overview
-- 博客总览：文章数、评论数、用户数
local function handle_overview()
    local posts_json, err = query("SELECT COUNT(*) as total FROM posts")
    if not posts_json then
        return error_response(500, err)
    end

    local comments_json, _ = query("SELECT COUNT(*) as total FROM comments")
    local users_json, _ = query("SELECT COUNT(*) as total FROM users")
    local published_json, _ = query("SELECT COUNT(*) as total FROM posts WHERE status = 'published'")

    -- 提取计数值
    local total_posts = tonumber(posts_json:match('"total":"?(%d+)"?')) or 0
    local total_comments = 0
    local total_users = 0
    local total_published = 0

    if comments_json then
        total_comments = tonumber(comments_json:match('"total":"?(%d+)"?')) or 0
    end
    if users_json then
        total_users = tonumber(users_json:match('"total":"?(%d+)"?')) or 0
    end
    if published_json then
        total_published = tonumber(published_json:match('"total":"?(%d+)"?')) or 0
    end

    local data = '{'
        .. '"total_posts":' .. total_posts .. ','
        .. '"total_published":' .. total_published .. ','
        .. '"total_comments":' .. total_comments .. ','
        .. '"total_users":' .. total_users
        .. '}'

    return json_response(200, data)
end

-- GET /api/v1/plugins/stats/posts
-- 按状态统计文章数量
local function handle_posts_stats()
    local result, err = query("SELECT status, COUNT(*) as count FROM posts GROUP BY status")
    if not result then
        return error_response(500, err)
    end

    -- result 已经是 JSON 数组，直接包装
    return json_response(200, result)
end

-- GET /api/v1/plugins/stats/recent
-- 最近发布的 10 篇文章
local function handle_recent()
    local result, err = query(
        "SELECT title, slug, view_count, published_at "
        .. "FROM posts WHERE status = 'published' "
        .. "ORDER BY published_at DESC LIMIT 10"
    )
    if not result then
        return error_response(500, err)
    end

    return json_response(200, result)
end

-- GET /api/v1/plugins/stats/top
-- 浏览量最高的 10 篇文章
local function handle_top()
    local result, err = query(
        "SELECT title, slug, view_count "
        .. "FROM posts WHERE status = 'published' "
        .. "ORDER BY view_count DESC LIMIT 10"
    )
    if not result then
        return error_response(500, err)
    end

    return json_response(200, result)
end

-- ── 路由匹配 ──────────────────────────────────────────────────

-- 简单的路由表：路径后缀 → 处理函数
local routes = {
    ["overview"] = handle_overview,
    ["posts"]    = handle_posts_stats,
    ["recent"]   = handle_recent,
    ["top"]      = handle_top,
}

-- ── Hook 入口 ─────────────────────────────────────────────────

Plugin = {
    handle_route = function(input)
        local path = input.path or ""
        local method = input.method or ""

        -- 只处理 GET 请求
        if method ~= "GET" then
            return nil
        end

        -- 从路径中提取最后一段作为路由名
        -- /api/v1/plugins/stats/overview → "overview"
        local route_name = path:match("/api/v1/plugins/stats/([^/]+)$")
        if not route_name then
            return nil
        end

        local handler = routes[route_name]
        if not handler then
            return error_response(404, "unknown endpoint: " .. route_name)
        end

        Host.log("info", "[blog-stats] " .. method .. " " .. path)
        return handler()
    end,
}
