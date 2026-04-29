-- Blog Stats API Plugin
--
-- 通过 routes 声明式注册自定义 REST 端点，暴露博客统计数据。
-- 使用 SDK v1 模块，通过 require("sdk") 导入工具函数。

local sdk = require("sdk")

Plugin = {}

Plugin.stats_overview = function(input)
    local posts = sdk.dbQuery("SELECT CAST(COUNT(*) AS TEXT) as total FROM posts")
    if not posts then return sdk.fail(500, "query failed") end

    local comments = sdk.dbQuery("SELECT CAST(COUNT(*) AS TEXT) as total FROM comments")
    local users = sdk.dbQuery("SELECT CAST(COUNT(*) AS TEXT) as total FROM users")
    local published = sdk.dbQuery("SELECT CAST(COUNT(*) AS TEXT) as total FROM posts WHERE status = 'published'")

    local total_posts = posts[1] and tonumber(posts[1].total) or 0
    local total_comments = comments and comments[1] and tonumber(comments[1].total) or 0
    local total_users = users and users[1] and tonumber(users[1].total) or 0
    local total_published = published and published[1] and tonumber(published[1].total) or 0

    sdk.logInfo("[blog-stats] GET overview")

    return sdk.ok({
        total_posts = total_posts,
        total_published = total_published,
        total_comments = total_comments,
        total_users = total_users,
    })
end

Plugin.stats_posts = function(input)
    local result = sdk.dbQuery("SELECT status, CAST(COUNT(*) AS TEXT) as count FROM posts GROUP BY status")
    if not result then return sdk.fail(500, "query failed") end
    return sdk.ok(result)
end

Plugin.stats_recent = function(input)
    local result = sdk.dbQuery(
        "SELECT title, slug, view_count, published_at "
        .. "FROM posts WHERE status = 'published' "
        .. "ORDER BY published_at DESC LIMIT 10"
    )
    if not result then return sdk.fail(500, "query failed") end
    return sdk.ok(result)
end

Plugin.stats_top = function(input)
    local result = sdk.dbQuery(
        "SELECT title, slug, view_count "
        .. "FROM posts WHERE status = 'published' "
        .. "ORDER BY view_count DESC LIMIT 10"
    )
    if not result then return sdk.fail(500, "query failed") end
    return sdk.ok(result)
end
