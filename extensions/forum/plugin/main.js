var Plugin = {};

function ok(result) {
    if (result && result._error) {
        return JSON.stringify({ status: result._status || 400, body: JSON.stringify({ ok: false, error: result._error }) });
    }
    return JSON.stringify({ status: 200, body: JSON.stringify({ ok: true, data: result }) });
}

function err(status, msg) {
    return { _error: msg, _status: status };
}

function parseBody(input) {
    try {
        if (typeof input === "string") {
            var parsed = JSON.parse(input);
            if (parsed && typeof parsed.body === "string" && parsed.body.charAt(0) === "{") {
                return JSON.parse(parsed.body);
            }
            return parsed;
        }
        if (input && input.body) return JSON.parse(input.body);
        return {};
    } catch (e) { return {}; }
}

function routeParam(input, index) {
    var obj = input;
    if (typeof input === "string") {
        try { obj = JSON.parse(input); } catch (e) { return ""; }
    }
    var path = (obj.path || "").replace(/\/+$/, "");
    var qIdx = path.indexOf("?");
    if (qIdx >= 0) path = path.substring(0, qIdx);
    var parts = path.split("/");
    return parts[parts.length - (index || 1)];
}

function genId() {
    return "xxxxxxxx-xxxx-7xxx-yxxx-xxxxxxxxxxxx".replace(/[xy]/g, function (c) {
        var r = (Math.random() * 16) | 0;
        var v = c === "x" ? r : (r & 0x3) | 0x8;
        return v.toString(16);
    });
}

function nowISO() {
    return new Date().toISOString();
}

function query(sql, params) {
    var result = Host.dbQuery(sql, params ? JSON.stringify(params) : null);
    if (!result || result.indexOf("error:") === 0) return null;
    return JSON.parse(result);
}

function exec(sql, params) {
    var result = Host.dbExecute(sql, params ? JSON.stringify(params) : null);
    return JSON.parse(result);
}

// ── Hooks ───────────────────────────────────────────────────

Plugin.on_content_creating = function (input) {
    var data = parseBody(input);
    var ct = data.content_type;
    var body = data.data || {};

    if (ct === "forum_reply") {
        var topicId = body.topic_id;
        if (topicId) {
            var topics = query("SELECT is_locked FROM forum_topics WHERE id = ?", [topicId]);
            if (topics && topics.length > 0 && topics[0].is_locked) {
                return JSON.stringify({ status: 400, body: JSON.stringify({ ok: false, error: "topic is locked" }) });
            }
        }
    }

    return JSON.stringify({ status: 200, body: JSON.stringify(data) });
};

Plugin.on_content_created = function (input) {
    var data = parseBody(input);
    var ct = data.content_type;
    var id = data.id;

    if (ct === "forum_topic") {
        var topics = query("SELECT board_id FROM forum_topics WHERE id = ?", [id]);
        if (topics && topics.length > 0) {
            var now = nowISO();
            exec("UPDATE forum_boards SET topic_count = topic_count + 1, post_count = post_count + 1, last_activity_at = ?, last_topic_id = ? WHERE id = ?",
                [now, id, topics[0].board_id]);
        }
    }

    if (ct === "forum_reply") {
        var replies = query("SELECT topic_id, author_id FROM forum_replies WHERE id = ?", [id]);
        if (replies && replies.length > 0) {
            var reply = replies[0];
            var now = nowISO();
            exec("UPDATE forum_topics SET reply_count = reply_count + 1, last_reply_at = ?, last_reply_user_id = ?, updated_at = ? WHERE id = ?",
                [now, reply.author_id, now, reply.topic_id]);

            var topics = query("SELECT board_id FROM forum_topics WHERE id = ?", [reply.topic_id]);
            if (topics && topics.length > 0) {
                exec("UPDATE forum_boards SET post_count = post_count + 1, last_activity_at = ? WHERE id = ?",
                    [now, topics[0].board_id]);
            }
        }
    }

    return ok(data);
};

Plugin.on_content_deleted = function (input) {
    var data = parseBody(input);
    var ct = data.content_type;
    var id = data.id;

    if (ct === "forum_topic") {
        var topics = query("SELECT board_id FROM forum_topics WHERE id = ?", [id]);
        if (topics && topics.length > 0) {
            exec("UPDATE forum_boards SET topic_count = CASE WHEN topic_count > 0 THEN topic_count - 1 ELSE 0 END, post_count = CASE WHEN post_count > 0 THEN post_count - 1 ELSE 0 END WHERE id = ?",
                [topics[0].board_id]);
        }
    }

    if (ct === "forum_reply") {
        var replies = query("SELECT topic_id FROM forum_replies WHERE id = ?", [id]);
        if (replies && replies.length > 0) {
            exec("UPDATE forum_topics SET reply_count = CASE WHEN reply_count > 0 THEN reply_count - 1 ELSE 0 END WHERE id = ?",
                [replies[0].topic_id]);
        }
    }

    return ok(data);
};

Plugin.on_content_viewed = function (input) {
    var data = parseBody(input);
    var ct = data.content_type;
    var id = data.id;

    if (ct === "forum_topic") {
        exec("UPDATE forum_topics SET view_count = view_count + 1 WHERE id = ?", [id]);
    }

    return ok(data);
};

// ── GET /boards/:slug/topics ────────────────────────────────

Plugin.listBoardTopics = function (input) {
    var slug = routeParam(input, 2);
    var page = 1;
    var pageSize = 20;
    if (page < 1) page = 1;
    if (pageSize < 1 || pageSize > 100) pageSize = 20;
    var offset = (page - 1) * pageSize;

    var boards = query("SELECT id FROM forum_boards WHERE slug = ?", [slug]);
    if (!boards || boards.length === 0) return ok(err(404, "board not found for slug: " + slug));
    var boardId = boards[0].id;

    var totalResult = query("SELECT COUNT(*) as cnt FROM forum_topics WHERE board_id = ?", [boardId]);
    var total = (totalResult && totalResult[0]) ? parseInt(totalResult[0].cnt, 10) : 0;

    var rows = query(
        "SELECT id, title, slug, author_id, reply_count, view_count, is_pinned, is_locked, is_solved, " +
        "last_reply_at, last_reply_user_id, tags, created_at " +
        "FROM forum_topics WHERE board_id = ? " +
        "ORDER BY is_pinned DESC, last_reply_at DESC, created_at DESC " +
        "LIMIT ? OFFSET ?",
        [boardId, pageSize, offset]
    );

    return ok({ items: rows || [], total: total, page: page, page_size: pageSize, board_id: boardId });
};

// ── PUT /replies/:id/accept ─────────────────────────────────

Plugin.acceptAnswer = function (input) {
    var replyId = routeParam(input, 2);
    var data = parseBody(input);
    var userId = data.user_id;
    if (!userId) return ok(err(400, "user_id required"));

    var replies = query("SELECT id, topic_id, author_id FROM forum_replies WHERE id = ?", [replyId]);
    if (!replies || replies.length === 0) return ok(err(404, "reply not found"));

    var reply = replies[0];
    var topics = query("SELECT id, author_id FROM forum_topics WHERE id = ?", [reply.topic_id]);
    if (!topics || topics.length === 0) return ok(err(404, "topic not found"));

    if (topics[0].author_id !== userId) return ok(err(403, "only topic author can accept answer"));

    exec("UPDATE forum_replies SET is_answer = 0 WHERE topic_id = ? AND is_answer = 1", [reply.topic_id]);
    exec("UPDATE forum_replies SET is_answer = 1, updated_at = ? WHERE id = ?", [nowISO(), replyId]);
    exec("UPDATE forum_topics SET is_solved = 1, updated_at = ? WHERE id = ?", [nowISO(), reply.topic_id]);

    return ok({ id: replyId, is_answer: true, topic_id: reply.topic_id });
};

// ── POST /vote ──────────────────────────────────────────────

Plugin.vote = function (input) {
    var data = parseBody(input);
    var userId = data.user_id;
    var targetType = data.target_type;
    var targetId = data.target_id;
    var value = data.value || 1;
    if (value < -1) value = -1;
    if (value > 1) value = 1;

    if (!userId) return ok(err(400, "user_id required"));
    if (!targetType) return ok(err(400, "target_type required"));
    if (!targetId) return ok(err(400, "target_id required"));
    if (targetType !== "topic" && targetType !== "reply") return ok(err(400, "target_type must be topic or reply"));

    var existing = query("SELECT id, value FROM forum_votes WHERE target_type = ? AND target_id = ? AND user_id = ?", [targetType, targetId, userId]);
    if (existing && existing.length > 0) {
        var oldValue = parseInt(existing[0].value, 10);
        var diff = value - oldValue;
        if (diff === 0) return ok(err(400, "already voted"));
        exec("UPDATE forum_votes SET value = ?, updated_at = ? WHERE id = ?", [value, nowISO(), existing[0].id]);
        updateVoteCount(targetType, targetId, diff);
    } else {
        var id = genId();
        var now = nowISO();
        exec("INSERT INTO forum_votes (id, tenant_id, target_type, target_id, user_id, value, created_at, updated_at) VALUES (?, 'default', ?, ?, ?, ?, ?, ?)",
            [id, targetType, targetId, userId, value, now, now]);
        updateVoteCount(targetType, targetId, value);
    }

    return ok({ target_type: targetType, target_id: targetId, value: value });
};

// ── DELETE /vote ────────────────────────────────────────────

Plugin.unvote = function (input) {
    var data = parseBody(input);
    var userId = data.user_id;
    var targetType = data.target_type;
    var targetId = data.target_id;

    if (!userId) return ok(err(400, "user_id required"));
    if (!targetType) return ok(err(400, "target_type required"));
    if (!targetId) return ok(err(400, "target_id required"));

    var existing = query("SELECT id, value FROM forum_votes WHERE target_type = ? AND target_id = ? AND user_id = ?", [targetType, targetId, userId]);
    if (!existing || existing.length === 0) return ok(err(404, "vote not found"));

    var oldValue = parseInt(existing[0].value, 10);
    exec("DELETE FROM forum_votes WHERE id = ?", [existing[0].id]);
    updateVoteCount(targetType, targetId, -oldValue);

    return ok({ removed: true });
};

function updateVoteCount(targetType, targetId, diff) {
    var table = targetType === "topic" ? "forum_topics" : "forum_replies";
    exec("UPDATE " + table + " SET vote_count = vote_count + ? WHERE id = ?", [diff, targetId]);
}

// ── Polls ───────────────────────────────────────────────────

Plugin.createPoll = function (input) {
    var data = parseBody(input);
    var userId = data.user_id;
    var topicId = data.topic_id;
    var question = (data.question || "").trim();
    var options = data.options || [];
    var maxChoices = data.max_choices || 1;

    if (!userId) return ok(err(400, "user_id required"));
    if (!topicId) return ok(err(400, "topic_id required"));
    if (!question) return ok(err(400, "question required"));
    if (!options || options.length < 2) return ok(err(400, "at least 2 options required"));
    if (options.length > 20) return ok(err(400, "too many options (max 20)"));
    if (maxChoices < 1) maxChoices = 1;
    if (maxChoices > options.length) maxChoices = options.length;

    var topics = query("SELECT id, author_id FROM forum_topics WHERE id = ?", [topicId]);
    if (!topics || topics.length === 0) return ok(err(404, "topic not found"));
    if (topics[0].author_id !== userId) return ok(err(403, "only topic author can create poll"));

    var existing = query("SELECT id FROM forum_polls WHERE topic_id = ?", [topicId]);
    if (existing && existing.length > 0) return ok(err(400, "poll already exists for this topic"));

    var pollId = genId();
    var now = nowISO();
    exec("INSERT INTO forum_polls (id, tenant_id, topic_id, question, max_choices, is_closed, created_at, updated_at) VALUES (?, 'default', ?, ?, ?, 0, ?, ?)",
        [pollId, topicId, question, maxChoices, now, now]);

    var createdOptions = [];
    for (var i = 0; i < options.length; i++) {
        var optText = (options[i] || "").trim();
        if (!optText) continue;
        var optId = genId();
        exec("INSERT INTO forum_poll_options (id, tenant_id, poll_id, text, vote_count, sort_order) VALUES (?, 'default', ?, ?, 0, ?)",
            [optId, pollId, optText, i]);
        createdOptions.push({ id: optId, text: optText, vote_count: 0, sort_order: i });
    }

    return ok({
        id: pollId,
        topic_id: topicId,
        question: question,
        max_choices: maxChoices,
        is_closed: false,
        options: createdOptions,
        total_votes: 0,
        user_votes: [],
        created_at: now
    });
};

Plugin.getPoll = function (input) {
    var topicId = routeParam(input, 1);
    var obj = input;
    if (typeof input === "string") { try { obj = JSON.parse(input); } catch (e) {} }
    var fullPath = obj.path || "";
    var qsIdx = fullPath.indexOf("?");
    var userId = "";
    if (qsIdx >= 0) {
        var qs = fullPath.substring(qsIdx + 1);
        var pairs = qs.split("&");
        for (var p = 0; p < pairs.length; p++) {
            if (pairs[p].indexOf("user_id=") === 0) {
                userId = decodeURIComponent(pairs[p].substring(8));
            }
        }
    }

    var polls = query("SELECT id, topic_id, question, max_choices, is_closed, created_at FROM forum_polls WHERE topic_id = ?", [topicId]);
    if (!polls || polls.length === 0) return ok(null);

    var poll = polls[0];
    var options = query("SELECT id, text, CAST(vote_count AS TEXT) as vote_count, CAST(sort_order AS TEXT) as sort_order FROM forum_poll_options WHERE poll_id = ? ORDER BY sort_order", [poll.id]);

    var totalResult = query("SELECT CAST(SUM(vote_count) AS TEXT) as total FROM forum_poll_options WHERE poll_id = ?", [poll.id]);
    var totalVotes = (totalResult && totalResult[0] && totalResult[0].total) ? parseInt(totalResult[0].total, 10) : 0;

    var userVotes = [];
    if (userId) {
        var votes = query("SELECT option_id FROM forum_poll_votes WHERE poll_id = ? AND user_id = ?", [poll.id, userId]);
        if (votes) {
            for (var i = 0; i < votes.length; i++) {
                userVotes.push(votes[i].option_id);
            }
        }
    }

    var fixedOptions = [];
    if (options) {
        for (var oi = 0; oi < options.length; oi++) {
            fixedOptions.push({
                id: options[oi].id,
                text: options[oi].text,
                vote_count: parseInt(options[oi].vote_count, 10) || 0,
                sort_order: parseInt(options[oi].sort_order, 10) || 0
            });
        }
    }

    return ok({
        id: poll.id,
        topic_id: poll.topic_id,
        question: poll.question,
        max_choices: parseInt(poll.max_choices, 10) || 1,
        is_closed: parseInt(poll.is_closed, 10) === 1,
        options: fixedOptions,
        total_votes: totalVotes,
        user_votes: userVotes,
        created_at: poll.created_at
    });
};

Plugin.castVote = function (input) {
    var pollId = routeParam(input, 2);
    var data = parseBody(input);
    var userId = data.user_id;
    var optionIds = data.option_ids || [];

    if (!userId) return ok(err(400, "user_id required"));
    if (!optionIds || optionIds.length === 0) return ok(err(400, "option_ids required"));

    var polls = query("SELECT id, topic_id, question, max_choices, is_closed FROM forum_polls WHERE id = ?", [pollId]);
    if (!polls || polls.length === 0) return ok(err(404, "poll not found"));

    var poll = polls[0];
    if (parseInt(poll.is_closed, 10) === 1) return ok(err(400, "poll is closed"));

    var maxChoices = parseInt(poll.max_choices, 10);
    if (optionIds.length > maxChoices) return ok(err(400, "too many choices (max " + maxChoices + ")"));

    var existingVotes = query("SELECT option_id FROM forum_poll_votes WHERE poll_id = ? AND user_id = ?", [pollId, userId]);
    if (existingVotes && existingVotes.length > 0) return ok(err(400, "already voted"));

    for (var i = 0; i < optionIds.length; i++) {
        var optId = optionIds[i];
        var opts = query("SELECT id FROM forum_poll_options WHERE id = ? AND poll_id = ?", [optId, pollId]);
        if (!opts || opts.length === 0) return ok(err(400, "option not found: " + optId));

        var voteId = genId();
        var now = nowISO();
        exec("INSERT INTO forum_poll_votes (id, tenant_id, poll_id, option_id, user_id, created_at, updated_at) VALUES (?, 'default', ?, ?, ?, ?, ?)",
            [voteId, pollId, optId, userId, now, now]);
        exec("UPDATE forum_poll_options SET vote_count = vote_count + 1 WHERE id = ?", [optId]);
    }

    return ok({ poll_id: pollId, voted_options: optionIds });
};

Plugin.deletePoll = function (input) {
    var pollId = routeParam(input, 1);
    var data = parseBody(input);
    var userId = data.user_id;

    if (!userId) return ok(err(400, "user_id required"));

    var polls = query("SELECT id, topic_id FROM forum_polls WHERE id = ?", [pollId]);
    if (!polls || polls.length === 0) return ok(err(404, "poll not found"));

    var topics = query("SELECT author_id FROM forum_topics WHERE id = ?", [polls[0].topic_id]);
    if (!topics || topics.length === 0 || topics[0].author_id !== userId) return ok(err(403, "only topic author can delete poll"));

    exec("DELETE FROM forum_poll_votes WHERE poll_id = ?", [pollId]);
    exec("DELETE FROM forum_poll_options WHERE poll_id = ?", [pollId]);
    exec("DELETE FROM forum_polls WHERE id = ?", [pollId]);

    return ok({ deleted: true });
};
