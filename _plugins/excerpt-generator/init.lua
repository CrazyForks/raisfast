Plugin = {
    on_post_creating = function(input)
        if not input.excerpt or input.excerpt == "" then
            local plain = input.content
                :gsub("```[\0-\255]*?```", "")
                :gsub("[#*_`]", "")
                :gsub("%s+", " ")
            plain = plain:match("^%s*(.-)%s*$") or plain

            if #plain > 200 then
                input.excerpt = plain:sub(1, 200) .. "..."
            else
                input.excerpt = plain
            end
        end

        local env = Host.getConfig("app.env")
        if env == "development" then
            Host.log("info", "excerpt generated in dev mode")
        end

        return input
    end,
}
