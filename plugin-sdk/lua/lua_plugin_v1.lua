local M = {}
M.SDK_VERSION = "1.0.0"

function M.dbQuery(sql, params)
    local paramsJson = params and Host.jsonEncode(params) or nil
    local result = Host.dbQuery(sql, paramsJson)
    if not result then error("query returned no result") end
    if result:sub(1, 6) == "error:" then error(result:sub(7)) end
    return Host.jsonDecode(result)
end

function M.dbExec(sql, params)
    local paramsJson = params and Host.jsonEncode(params) or nil
    local result = Host.dbExecute(sql, paramsJson)
    return Host.jsonDecode(result)
end

function M.dbBegin()
    local result = Host.jsonDecode(Host.dbBegin())
    if not result.ok then error("dbBegin failed") end
    return result
end

function M.dbCommit()
    local result = Host.jsonDecode(Host.dbCommit())
    if not result.ok then error("dbCommit failed") end
    return result
end

function M.dbRollback()
    return Host.jsonDecode(Host.dbRollback())
end

function M.httpGet(url)
    return Host.httpGet(url)
end

function M.httpGetJson(url)
    local result = Host.httpGet(url)
    if not result then return nil end
    local ok, decoded = pcall(Host.jsonDecode, result)
    return ok and decoded or nil
end

function M.httpPost(url, body)
    local jsonBody = type(body) == "string" and body or Host.jsonEncode(body)
    return Host.httpPost(url, jsonBody)
end

function M.httpPostJson(url, body)
    local jsonBody = type(body) == "string" and body or Host.jsonEncode(body)
    local result = Host.httpPost(url, jsonBody)
    if not result then return nil end
    local ok, decoded = pcall(Host.jsonDecode, result)
    return ok and decoded or nil
end

function M.configGet(key) return Host.getConfig(key) end

function M.storeGet(key) return Host.getData(key) end
function M.storeSet(key, value) return Host.setData(key, value) end

function M.vfsRead(path) return Host.fsRead(path) end
function M.vfsWrite(path, content) return Host.fsWrite(path, content) end
function M.vfsDelete(path) return Host.fsDelete(path) end
function M.vfsExists(path) return Host.fsExists(path) end
function M.vfsList(path)
    local result = Host.fsList(path)
    if not result then return nil end
    local list = {}
    for part in result:gmatch("[^,]+") do
        table.insert(list, part)
    end
    return list
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
            parsed = Host.jsonDecode(input)
        end
        if not field or field == "" then return parsed end
        local val = parsed
        for part in field:gmatch("[^.]+") do
            if type(val) ~= "table" then return nil end
            val = val[part]
        end
        if type(val) == "string" then
            local decodeOk, decoded = pcall(Host.jsonDecode, val)
            if decodeOk then return decoded end
            return val
        end
        return val
    end)
    return ok and result or nil
end

function M.logInfo(msg) Host.log("info", msg) end
function M.logWarn(msg) Host.log("warn", msg) end
function M.logError(msg) Host.log("error", msg) end

function M.newId() return Host.newId() end

function M.eventEmit(eventType, data)
    local dataStr = type(data) == "string" and data or Host.jsonEncode(data)
    return Host.emitEvent(eventType, dataStr)
end

_sdk_module = M
