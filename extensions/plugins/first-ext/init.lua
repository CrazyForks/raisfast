-- Blog Stats API Plugin
--
-- 通过 routes 声明式注册自定义 REST 端点，暴露博客统计数据。
-- 使用 SDK v1 模块，通过 require("sdk") 导入工具函数。

local sdk = require("sdk")

Plugin = {}

Plugin.stats_overview = function(input)
    local total_posts = sdk.dbCount("posts")
    local total_comments = sdk.dbCount("comments")
    local total_users = sdk.dbCount("users")
    local total_published = sdk.dbCount("posts", { status = "published" })

    sdk.logInfo("[blog-stats] GET overview")

    return sdk.ok({
        total_posts = total_posts,
        total_published = total_published,
        total_comments = total_comments,
        total_users = total_users,
    })
end

Plugin.stats_posts = function(input)
    local result = sdk.dbGroupBy("posts", {
        group_by = "status",
        count = true
    })
    return sdk.ok(result.data or {})
end

Plugin.stats_recent = function(input)
    local result = sdk.dbFetchAll("posts", { status = "published" }, { order_by = "published_at DESC", limit = 10 })
    return sdk.ok(result.data or {})
end

Plugin.stats_top = function(input)
    local result = sdk.dbFetchAll("posts", { status = "published" }, { order_by = "view_count DESC", limit = 10 })
    return sdk.ok(result.data or {})
end
