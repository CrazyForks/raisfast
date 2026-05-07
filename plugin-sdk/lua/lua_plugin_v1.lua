local M = {}
M.SDK_VERSION = "1.0.0"

function M.dbPh(idx)
    return RaisFastHost.dbPh(idx)
end

function M.dbQuery(sql, params)
    local paramsJson = params and RaisFastHost.jsonEncode(params) or "[]"
    local result = RaisFastHost.dbQuery(sql, paramsJson)
    if not result then error("query returned no result") end
    if result:sub(1, 6) == "error:" then error(result:sub(7)) end
    return RaisFastHost.jsonDecode(result)
end

function M.dbExec(sql, params)
    local paramsJson = params and RaisFastHost.jsonEncode(params) or "[]"
    local result = RaisFastHost.dbExecute(sql, paramsJson)
    return RaisFastHost.jsonDecode(result)
end

function M.dbBegin()
    local result = RaisFastHost.jsonDecode(RaisFastHost.dbBegin())
    if not result.ok then error("dbBegin failed") end
    return result
end

function M.dbCommit()
    local result = RaisFastHost.jsonDecode(RaisFastHost.dbCommit())
    if not result.ok then error("dbCommit failed") end
    return result
end

function M.dbRollback()
    return RaisFastHost.jsonDecode(RaisFastHost.dbRollback())
end

function M.httpGet(url)
    return RaisFastHost.httpGet(url)
end

function M.httpGetJson(url)
    local result = RaisFastHost.httpGet(url)
    if not result then return nil end
    local ok, decoded = pcall(RaisFastHost.jsonDecode, result)
    return ok and decoded or nil
end

function M.httpPost(url, body)
    local jsonBody = type(body) == "string" and body or RaisFastHost.jsonEncode(body)
    return RaisFastHost.httpPost(url, jsonBody)
end

function M.httpPostJson(url, body)
    local jsonBody = type(body) == "string" and body or RaisFastHost.jsonEncode(body)
    local result = RaisFastHost.httpPost(url, jsonBody)
    if not result then return nil end
    local ok, decoded = pcall(RaisFastHost.jsonDecode, result)
    return ok and decoded or nil
end

function M.configGet(key) return RaisFastHost.getConfig(key) end

function M.storeGet(key) return RaisFastHost.getData(key) end
function M.storeSet(key, value) return RaisFastHost.setData(key, value) end

function M.vfsRead(path) return RaisFastHost.vfsRead(path) end
function M.vfsWrite(path, content) return RaisFastHost.vfsWrite(path, content) end
function M.vfsDelete(path) return RaisFastHost.vfsDelete(path) end
function M.vfsExists(path) return RaisFastHost.vfsExists(path) end
function M.vfsList(path)
    local result = RaisFastHost.vfsList(path)
    if not result then return nil end
    local list = {}
    for part in result:gmatch("[^,]+") do
        table.insert(list, part)
    end
    return list
end

function M.vfsStat(path)
    local result = RaisFastHost.vfsStat(path)
    if not result then return nil end
    local ok, decoded = pcall(RaisFastHost.jsonDecode, result)
    return ok and decoded or nil
end

function M.getPost(slug)
    local result = RaisFastHost.getPost(slug)
    if not result then return nil end
    local ok, decoded = pcall(RaisFastHost.jsonDecode, result)
    return ok and decoded or nil
end

function M.ok(data)
    return data
end

function M.fail(status, msg)
    return { __plugin_error = true, __status = status, __message = tostring(msg) }
end

function M.extractJson(input, field)
    local ok, result = pcall(function()
        local parsed = input
        if type(input) == "string" then
            parsed = RaisFastHost.jsonDecode(input)
        end
        if not field or field == "" then return parsed end
        local val = parsed
        for part in field:gmatch("[^.]+") do
            if type(val) ~= "table" then return nil end
            val = val[part]
        end
        if type(val) == "string" then
            local decodeOk, decoded = pcall(RaisFastHost.jsonDecode, val)
            if decodeOk then return decoded end
            return val
        end
        return val
    end)
    return ok and result or nil
end

function M.logInfo(msg) RaisFastHost.log("info", msg) end
function M.logWarn(msg) RaisFastHost.log("warn", msg) end
function M.logError(msg) RaisFastHost.log("error", msg) end

function M.newId() return RaisFastHost.newId() end

function M.eventEmit(eventType, data)
    local dataStr = type(data) == "string" and data or RaisFastHost.jsonEncode(data)
    return RaisFastHost.emitEvent(eventType, dataStr)
end

_sdk_module = M
