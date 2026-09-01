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

function M.dbInsert(table, data, options)
    local dataStr = type(data) == "string" and data or RaisFastHost.jsonEncode(data)
    local optStr = (options == nil) and "{}" or (type(options) == "string" and options or RaisFastHost.jsonEncode(options))
    local result = RaisFastHost.jsonDecode(RaisFastHost.dbInsert(table, dataStr, optStr))
    if result.error then error(result.error) end
    return result
end

function M.dbFetchOne(table, where, options)
    local whereStr = (where == nil) and "{}" or (type(where) == "string" and where or RaisFastHost.jsonEncode(where))
    local optStr = (options == nil) and "{}" or (type(options) == "string" and options or RaisFastHost.jsonEncode(options))
    local result = RaisFastHost.jsonDecode(RaisFastHost.dbFetchOne(table, whereStr, optStr))
    if result.error then error(result.error) end
    return result.data
end

function M.dbFetchAll(table, where, options)
    local whereStr = (where == nil) and "{}" or (type(where) == "string" and where or RaisFastHost.jsonEncode(where))
    local optStr = (options == nil) and "{}" or (type(options) == "string" and options or RaisFastHost.jsonEncode(options))
    local result = RaisFastHost.jsonDecode(RaisFastHost.dbFetchAll(table, whereStr, optStr))
    if result.error then error(result.error) end
    return result
end

function M.dbUpdate(table, data, where, options)
    local dataStr = type(data) == "string" and data or RaisFastHost.jsonEncode(data)
    local whereStr = (where == nil) and "{}" or (type(where) == "string" and where or RaisFastHost.jsonEncode(where))
    local optStr = (options == nil) and "{}" or (type(options) == "string" and options or RaisFastHost.jsonEncode(options))
    local result = RaisFastHost.jsonDecode(RaisFastHost.dbUpdate(table, dataStr, whereStr, optStr))
    if result.error then error(result.error) end
    return result
end

function M.dbDelete(table, where, options)
    local whereStr = (where == nil) and "{}" or (type(where) == "string" and where or RaisFastHost.jsonEncode(where))
    local optStr = (options == nil) and "{}" or (type(options) == "string" and options or RaisFastHost.jsonEncode(options))
    local result = RaisFastHost.jsonDecode(RaisFastHost.dbDelete(table, whereStr, optStr))
    if result.error then error(result.error) end
    return result
end

function M.dbCount(table, where, options)
    local whereStr = (where == nil) and "{}" or (type(where) == "string" and where or RaisFastHost.jsonEncode(where))
    local optStr = (options == nil) and "{}" or (type(options) == "string" and options or RaisFastHost.jsonEncode(options))
    local result = RaisFastHost.jsonDecode(RaisFastHost.dbCount(table, whereStr, optStr))
    if result.error then error(result.error) end
    return result.count
end

function M.dbIncrement(table, columns, where, options)
    local colStr = type(columns) == "string" and columns or RaisFastHost.jsonEncode(columns)
    local whereStr = (where == nil) and "{}" or (type(where) == "string" and where or RaisFastHost.jsonEncode(where))
    local optStr = (options == nil) and "{}" or (type(options) == "string" and options or RaisFastHost.jsonEncode(options))
    local result = RaisFastHost.jsonDecode(RaisFastHost.dbIncrement(table, colStr, whereStr, optStr))
    if result.error then error(result.error) end
    return result
end

function M.dbSum(table, column, where, options)
    local whereStr = (where == nil) and "{}" or (type(where) == "string" and where or RaisFastHost.jsonEncode(where))
    local optStr = (options == nil) and "{}" or (type(options) == "string" and options or RaisFastHost.jsonEncode(options))
    local result = RaisFastHost.jsonDecode(RaisFastHost.dbSum(table, column, whereStr, optStr))
    if result.error then error(result.error) end
    return result.sum
end

function M.dbGroupBy(table, options)
    local optStr = (options == nil) and "{}" or (type(options) == "string" and options or RaisFastHost.jsonEncode(options))
    local result = RaisFastHost.jsonDecode(RaisFastHost.dbGroupBy(table, optStr))
    if result.error then error(result.error) end
    return result
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

function M.callApi(clientKey, op, input)
    local jsonBody = input == nil and "{}" or (type(input) == "string" and input or RaisFastHost.jsonEncode(input))
    local ok, decoded = pcall(RaisFastHost.jsonDecode, RaisFastHost.callApi(clientKey, op, jsonBody))
    if not ok then return nil, "decode failed" end
    if decoded and decoded.error then
        return nil, decoded.error
    end
    return decoded
end

-- Content-type host API (group-aware: 'group/plural', plural, table name)
function M.ctFind(ct, query)
    local q = query == nil and "{}" or (type(query) == "string" and query or RaisFastHost.jsonEncode(query))
    local decoded = RaisFastHost.jsonDecode(RaisFastHost.ctFind(ct, q))
    if decoded.error then error(decoded.error) end
    return decoded
end

function M.ctGet(ct, id)
    local result = RaisFastHost.ctGet(ct, tostring(id))
    if result == nil or result == "null" then return nil end
    return RaisFastHost.jsonDecode(result)
end

function M.ctCreate(ct, data)
    local dataStr = data == nil and "{}" or (type(data) == "string" and data or RaisFastHost.jsonEncode(data))
    local decoded = RaisFastHost.jsonDecode(RaisFastHost.ctCreate(ct, dataStr))
    if decoded and decoded.error then error(decoded.error) end
    return decoded
end

function M.ctUpdate(ct, id, data)
    local dataStr = data == nil and "{}" or (type(data) == "string" and data or RaisFastHost.jsonEncode(data))
    local decoded = RaisFastHost.jsonDecode(RaisFastHost.ctUpdate(ct, tostring(id), dataStr))
    if decoded and decoded.error then error(decoded.error) end
    return decoded
end

-- Job / integration host API
function M.jobEnqueue(jobType, payload, opts)
    local p = payload == nil and "{}" or (type(payload) == "string" and payload or RaisFastHost.jsonEncode(payload))
    local o = opts == nil and "{}" or (type(opts) == "string" and opts or RaisFastHost.jsonEncode(opts))
    local decoded = RaisFastHost.jsonDecode(RaisFastHost.jobEnqueue(jobType, p, o))
    if decoded.error then error(decoded.error) end
    return decoded
end

function M.getReceipt(traceId)
    local result = RaisFastHost.getReceipt(tostring(traceId))
    if result == nil or result == "null" then return nil end
    return RaisFastHost.jsonDecode(result)
end

-- Integration channel host API (app-scoped; channel-app-ownership.md §4.2)
function M.channelList()
    return RaisFastHost.jsonDecode(RaisFastHost.channelList())
end

function M.channelCreate(data)
    local dataStr = data == nil and "{}" or (type(data) == "string" and data or RaisFastHost.jsonEncode(data))
    local decoded = RaisFastHost.jsonDecode(RaisFastHost.channelCreate(dataStr))
    if decoded and decoded.error then error(decoded.error) end
    return decoded
end

function M.channelUpdate(id, data)
    local dataStr = data == nil and "{}" or (type(data) == "string" and data or RaisFastHost.jsonEncode(data))
    local decoded = RaisFastHost.jsonDecode(RaisFastHost.channelUpdate(tostring(id), dataStr))
    if decoded and decoded.error then error(decoded.error) end
    return decoded
end

function M.channelDelete(id)
    local decoded = RaisFastHost.jsonDecode(RaisFastHost.channelDelete(tostring(id)))
    if decoded and decoded.error then error(decoded.error) end
    return decoded
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
