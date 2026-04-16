-- Page Cache Plugin
--
-- 使用虚拟文件系统 (VFS) 缓存渲染后的文章内容。
-- 完整演示所有文件系统 Host API：
--   fsRead / fsWrite / fsDelete / fsExists / fsList / fsStat
--
-- 缓存目录结构：
--   cache/
--     index.json          -- 缓存索引（slug → 标题映射）
--     posts/
--       <slug>.txt        -- 文章内容缓存
--   stats.json            -- 统计信息

-- 工具函数：安全地 JSON 编解码（用字符串模拟）
local function json_encode_table(t)
    local parts = {}
    for k, v in pairs(t) do
        local val
        if type(v) == "string" then
            val = '"' .. v:gsub('"', '\\"') .. '"'
        elseif type(v) == "number" then
            val = tostring(v)
        elseif type(v) == "boolean" then
            val = v and "true" or "false"
        else
            val = '""'
        end
        table.insert(parts, '"' .. k .. '":' .. val)
    end
    return "{" .. table.concat(parts, ",") .. "}"
end

-- 读取缓存索引
local function load_index()
    local content = Host.fsRead("cache/index.json")
    if not content then
        return {}
    end
    local idx = {}
    for slug, title in content:gmatch('"([^"]+)":"([^"]*)"') do
        idx[slug] = title
    end
    return idx
end

-- 写入缓存索引
local function save_index(idx)
    local parts = {}
    for slug, title in pairs(idx) do
        table.insert(parts, '"' .. slug .. '":"' .. title .. '"')
    end
    Host.fsWrite("cache/index.json", "{" .. table.concat(parts, ",") .. "}")
end

-- 更新统计信息
local function update_stats(action, slug)
    local stat_raw = Host.fsRead("stats.json")
    local stats = { writes = 0, deletes = 0, reads = 0, hits = 0 }
    if stat_raw then
        -- 简易解析
        stats.writes = tonumber(stat_raw:match('"writes":(%d+)')) or 0
        stats.deletes = tonumber(stat_raw:match('"deletes":(%d+)')) or 0
        stats.reads = tonumber(stat_raw:match('"reads":(%d+)')) or 0
        stats.hits = tonumber(stat_raw:match('"hits":(%d+)')) or 0
    end

    if action == "write" then
        stats.writes = stats.writes + 1
    elseif action == "delete" then
        stats.deletes = stats.deletes + 1
    elseif action == "read" then
        stats.reads = stats.reads + 1
    elseif action == "hit" then
        stats.hits = stats.hits + 1
    end

    Host.fsWrite("stats.json", json_encode_table(stats))
    Host.log("info", "[page-cache] stats updated: " .. action .. " (slug=" .. (slug or "") .. ")")
end

-- 将文章内容写入缓存文件
local function cache_post(slug, title, content)
    local path = "cache/posts/" .. slug .. ".txt"
    local header = "# " .. title .. "\n\n"
    Host.fsWrite(path, header .. content)
    Host.log("info", "[page-cache] cached post: " .. slug)

    -- 更新索引
    local idx = load_index()
    idx[slug] = title
    save_index(idx)

    update_stats("write", slug)
end

-- 删除文章缓存
local function remove_cache(slug)
    local path = "cache/posts/" .. slug .. ".txt"
    if Host.fsExists(path) then
        Host.fsDelete(path)
        Host.log("info", "[page-cache] removed cache: " .. slug)
    end

    -- 更新索引
    local idx = load_index()
    idx[slug] = nil
    save_index(idx)

    update_stats("delete", slug)
end

-- 列出所有缓存的文章（演示 fsList）
local function list_cached_posts()
    local entries = Host.fsList("cache/posts")
    if not entries then
        Host.log("info", "[page-cache] no cached posts directory")
        return
    end

    Host.log("info", "[page-cache] cached posts listing:")
    for _, name in ipairs(entries) do
        -- 获取文件信息（演示 fsStat）
        local stat_json = Host.fsStat("cache/posts/" .. name)
        if stat_json then
            local size = stat_json:match('"size":(%d+)')
            Host.log("info", "  " .. name .. " (" .. (size or "?") .. " bytes)")
        else
            Host.log("info", "  " .. name)
        end
    end
end

-- 演示 fsExists：检查缓存是否存在
local function has_cache(slug)
    return Host.fsExists("cache/posts/" .. slug .. ".txt")
end

-- ============ Hook 处理函数 ============

Plugin = {
    -- 文章创建后 → 写入缓存
    on_post_created = function(input)
        local slug = input.slug or ""
        local title = input.title or ""
        local content = input.content or ""

        if slug ~= "" then
            cache_post(slug, title, content)
        end

        return input
    end,

    -- 文章更新后 → 刷新缓存
    on_post_updated = function(input)
        local slug = input.slug or ""
        local title = input.title or ""
        local content = input.content or ""

        if slug ~= "" then
            -- 先删除旧缓存再写入新内容
            remove_cache(slug)
            cache_post(slug, title, content)
            Host.log("info", "[page-cache] refreshed cache for: " .. slug)
        end

        return input
    end,

    -- 文章删除后 → 清除缓存
    on_post_deleted = function(input)
        local slug = input.slug or ""
        if slug ~= "" then
            remove_cache(slug)
        end

        -- 删除后展示剩余缓存列表（演示 fsList）
        list_cached_posts()

        return input
    end,

    -- 文章创建前 → 检查是否有同名缓存（演示 fsExists）
    on_post_creating = function(input)
        local slug = input.slug or ""

        if slug ~= "" and has_cache(slug) then
            Host.log("warn", "[page-cache] WARNING: post slug '" .. slug .. "' already has a cache entry!")
            Host.log("warn", "[page-cache] existing cache will be overwritten after creation")
        end

        -- 读取并打印统计信息（演示 fsRead）
        local stat_raw = Host.fsRead("stats.json")
        if stat_raw then
            Host.log("info", "[page-cache] current stats: " .. stat_raw)
        end

        return input
    end,
}
