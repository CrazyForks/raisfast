const Plugin = {};

const ok = (result) => {
    if (result?._error) {
        return JSON.stringify({ status: result._status || 400, body: JSON.stringify({ ok: false, error: result._error }) });
    }
    return JSON.stringify({ status: 200, body: JSON.stringify({ ok: true, data: result }) });
};

const err = (status, msg) => ({ _error: msg, _status: status });

const parseBody = (input) => {
    try {
        if (typeof input === "string") {
            const parsed = JSON.parse(input);
            if (parsed && typeof parsed.body === "string" && parsed.body.charAt(0) === "{") {
                return JSON.parse(parsed.body);
            }
            return parsed;
        }
        if (input?.body) return JSON.parse(input.body);
        return {};
    } catch (e) { return {}; }
};

const routeParam = (input, index) => {
    let obj = input;
    if (typeof input === "string") {
        try { obj = JSON.parse(input); } catch (e) { return ""; }
    }
    let path = (obj.path || "").replace(/\/+$/, "");
    const qIdx = path.indexOf("?");
    if (qIdx >= 0) path = path.substring(0, qIdx);
    const parts = path.split("/");
    return parts[parts.length - (index || 1)];
};

const genId = () => {
    return "xxxxxxxx-xxxx-7xxx-yxxx-xxxxxxxxxxxx".replace(/[xy]/g, (c) => {
        const r = (Math.random() * 16) | 0;
        const v = c === "x" ? r : (r & 0x3) | 0x8;
        return v.toString(16);
    });
};

const nowISO = () => new Date().toISOString();

const query = (sql, params) => {
    const result = Host.dbQuery(sql, params ? JSON.stringify(params) : null);
    if (!result || result.indexOf("error:") === 0) return null;
    return JSON.parse(result);
};

const exec = (sql, params) => {
    const result = Host.dbExecute(sql, params ? JSON.stringify(params) : null);
    return JSON.parse(result);
};

// ── Hooks ───────────────────────────────────────────────────

Plugin.on_content_creating = (input) => {
    const data = parseBody(input);
    const ct = data.content_type;
    const body = data.data || {};

    if (ct === "forum_reply") {
        const topicId = body.topic_id;
        if (topicId) {
            const topics = query("SELECT is_locked FROM forum_topics WHERE id = ?", [topicId]);
            if (topics?.length > 0 && topics[0].is_locked) {
                return JSON.stringify({ status: 400, body: JSON.stringify({ ok: false, error: "topic is locked" }) });
            }
        }
    }

    return JSON.stringify({ status: 200, body: JSON.stringify(data) });
};

Plugin.on_content_created = (input) => {
    const data = parseBody(input);
    const ct = data.content_type;
    const id = data.id;

    if (ct === "forum_topic") {
        const topics = query("SELECT board_id FROM forum_topics WHERE id = ?", [id]);
        if (topics?.length > 0) {
            const now = nowISO();
            exec("UPDATE forum_boards SET topic_count = topic_count + 1, post_count = post_count + 1, last_activity_at = ?, last_topic_id = ? WHERE id = ?",
                [now, id, topics[0].board_id]);
        }
    }

    if (ct === "forum_reply") {
        const replies = query("SELECT topic_id, author_id FROM forum_replies WHERE id = ?", [id]);
        if (replies?.length > 0) {
            const reply = replies[0];
            const now = nowISO();
            exec("UPDATE forum_topics SET reply_count = reply_count + 1, last_reply_at = ?, last_reply_user_id = ?, updated_at = ? WHERE id = ?",
                [now, reply.author_id, now, reply.topic_id]);

            const topics = query("SELECT board_id FROM forum_topics WHERE id = ?", [reply.topic_id]);
            if (topics?.length > 0) {
                exec("UPDATE forum_boards SET post_count = post_count + 1, last_activity_at = ? WHERE id = ?",
                    [now, topics[0].board_id]);
            }
        }
    }

    return ok(data);
};

Plugin.on_content_deleted = (input) => {
    const data = parseBody(input);
    const ct = data.content_type;
    const id = data.id;

    if (ct === "forum_topic") {
        const topics = query("SELECT board_id FROM forum_topics WHERE id = ?", [id]);
        if (topics?.length > 0) {
            exec("UPDATE forum_boards SET topic_count = CASE WHEN topic_count > 0 THEN topic_count - 1 ELSE 0 END, post_count = CASE WHEN post_count > 0 THEN post_count - 1 ELSE 0 END WHERE id = ?",
                [topics[0].board_id]);
        }
    }

    if (ct === "forum_reply") {
        const replies = query("SELECT topic_id FROM forum_replies WHERE id = ?", [id]);
        if (replies?.length > 0) {
            exec("UPDATE forum_topics SET reply_count = CASE WHEN reply_count > 0 THEN reply_count - 1 ELSE 0 END WHERE id = ?",
                [replies[0].topic_id]);
        }
    }

    return ok(data);
};

Plugin.on_content_viewed = (input) => {
    const data = parseBody(input);
    const ct = data.content_type;
    const id = data.id;

    if (ct === "forum_topic") {
        exec("UPDATE forum_topics SET view_count = view_count + 1 WHERE id = ?", [id]);
    }

    return ok(data);
};

// ── GET /boards/:slug/topics ────────────────────────────────

Plugin.listBoardTopics = (input) => {
    const slug = routeParam(input, 2);
    let page = 1;
    let pageSize = 20;
    if (page < 1) page = 1;
    if (pageSize < 1 || pageSize > 100) pageSize = 20;
    const offset = (page - 1) * pageSize;

    const boards = query("SELECT id FROM forum_boards WHERE slug = ?", [slug]);
    if (!boards || boards.length === 0) return ok(err(404, `board not found for slug: ${slug}`));
    const boardId = boards[0].id;

    const totalResult = query("SELECT COUNT(*) as cnt FROM forum_topics WHERE board_id = ?", [boardId]);
    const total = totalResult?.[0] ? parseInt(totalResult[0].cnt, 10) : 0;

    const rows = query(
        "SELECT id, title, slug, author_id, reply_count, view_count, is_pinned, is_locked, is_solved, " +
        "last_reply_at, last_reply_user_id, tags, created_at " +
        "FROM forum_topics WHERE board_id = ? " +
        "ORDER BY is_pinned DESC, last_reply_at DESC, created_at DESC " +
        "LIMIT ? OFFSET ?",
        [boardId, pageSize, offset]
    );

    return ok({ items: rows || [], total, page, page_size: pageSize, board_id: boardId });
};

// ── PUT /replies/:id/accept ─────────────────────────────────

Plugin.acceptAnswer = (input) => {
    const replyId = routeParam(input, 2);
    const data = parseBody(input);
    const userId = data.user_id;
    if (!userId) return ok(err(400, "user_id required"));

    const replies = query("SELECT id, topic_id, author_id FROM forum_replies WHERE id = ?", [replyId]);
    if (!replies || replies.length === 0) return ok(err(404, "reply not found"));

    const reply = replies[0];
    const topics = query("SELECT id, author_id FROM forum_topics WHERE id = ?", [reply.topic_id]);
    if (!topics || topics.length === 0) return ok(err(404, "topic not found"));

    if (topics[0].author_id !== userId) return ok(err(403, "only topic author can accept answer"));

    exec("UPDATE forum_replies SET is_answer = 0 WHERE topic_id = ? AND is_answer = 1", [reply.topic_id]);
    exec("UPDATE forum_replies SET is_answer = 1, updated_at = ? WHERE id = ?", [nowISO(), replyId]);
    exec("UPDATE forum_topics SET is_solved = 1, updated_at = ? WHERE id = ?", [nowISO(), reply.topic_id]);

    return ok({ id: replyId, is_answer: true, topic_id: reply.topic_id });
};

// ── POST /vote ──────────────────────────────────────────────

Plugin.vote = (input) => {
    const data = parseBody(input);
    let userId = data.user_id;
    const targetType = data.target_type;
    const targetId = data.target_id;
    let value = data.value || 1;
    if (value < -1) value = -1;
    if (value > 1) value = 1;

    if (!userId) return ok(err(400, "user_id required"));
    if (!targetType) return ok(err(400, "target_type required"));
    if (!targetId) return ok(err(400, "target_id required"));
    if (targetType !== "topic" && targetType !== "reply") return ok(err(400, "target_type must be topic or reply"));

    const existing = query("SELECT id, value FROM forum_votes WHERE target_type = ? AND target_id = ? AND user_id = ?", [targetType, targetId, userId]);
    if (existing?.length > 0) {
        const oldValue = parseInt(existing[0].value, 10);
        const diff = value - oldValue;
        if (diff === 0) return ok(err(400, "already voted"));
        exec("UPDATE forum_votes SET value = ?, updated_at = ? WHERE id = ?", [value, nowISO(), existing[0].id]);
        updateVoteCount(targetType, targetId, diff);
    } else {
        const id = genId();
        const now = nowISO();
        exec("INSERT INTO forum_votes (id, tenant_id, target_type, target_id, user_id, value, created_at, updated_at) VALUES (?, 'default', ?, ?, ?, ?, ?, ?)",
            [id, targetType, targetId, userId, value, now, now]);
        updateVoteCount(targetType, targetId, value);
    }

    return ok({ target_type: targetType, target_id: targetId, value });
};

// ── DELETE /vote ────────────────────────────────────────────

Plugin.unvote = (input) => {
    const data = parseBody(input);
    const userId = data.user_id;
    const targetType = data.target_type;
    const targetId = data.target_id;

    if (!userId) return ok(err(400, "user_id required"));
    if (!targetType) return ok(err(400, "target_type required"));
    if (!targetId) return ok(err(400, "target_id required"));

    const existing = query("SELECT id, value FROM forum_votes WHERE target_type = ? AND target_id = ? AND user_id = ?", [targetType, targetId, userId]);
    if (!existing || existing.length === 0) return ok(err(404, "vote not found"));

    const oldValue = parseInt(existing[0].value, 10);
    exec("DELETE FROM forum_votes WHERE id = ?", [existing[0].id]);
    updateVoteCount(targetType, targetId, -oldValue);

    return ok({ removed: true });
};

const updateVoteCount = (targetType, targetId, diff) => {
    const table = targetType === "topic" ? "forum_topics" : "forum_replies";
    exec(`UPDATE ${table} SET vote_count = vote_count + ? WHERE id = ?`, [diff, targetId]);
};

// ── Polls ───────────────────────────────────────────────────

Plugin.createPoll = (input) => {
    const data = parseBody(input);
    const userId = data.user_id;
    const topicId = data.topic_id;
    const question = (data.question || "").trim();
    const options = data.options || [];
    let maxChoices = data.max_choices || 1;

    if (!userId) return ok(err(400, "user_id required"));
    if (!topicId) return ok(err(400, "topic_id required"));
    if (!question) return ok(err(400, "question required"));
    if (!options || options.length < 2) return ok(err(400, "at least 2 options required"));
    if (options.length > 20) return ok(err(400, "too many options (max 20)"));
    if (maxChoices < 1) maxChoices = 1;
    if (maxChoices > options.length) maxChoices = options.length;

    const topics = query("SELECT id, author_id FROM forum_topics WHERE id = ?", [topicId]);
    if (!topics || topics.length === 0) return ok(err(404, "topic not found"));
    if (topics[0].author_id !== userId) return ok(err(403, "only topic author can create poll"));

    const existing = query("SELECT id FROM forum_polls WHERE topic_id = ?", [topicId]);
    if (existing?.length > 0) return ok(err(400, "poll already exists for this topic"));

    const pollId = genId();
    const now = nowISO();
    exec("INSERT INTO forum_polls (id, tenant_id, topic_id, question, max_choices, is_closed, created_at, updated_at) VALUES (?, 'default', ?, ?, ?, 0, ?, ?)",
        [pollId, topicId, question, maxChoices, now, now]);

    const createdOptions = [];
    for (let i = 0; i < options.length; i++) {
        const optText = (options[i] || "").trim();
        if (!optText) continue;
        const optId = genId();
        exec("INSERT INTO forum_poll_options (id, tenant_id, poll_id, text, vote_count, sort_order) VALUES (?, 'default', ?, ?, 0, ?)",
            [optId, pollId, optText, i]);
        createdOptions.push({ id: optId, text: optText, vote_count: 0, sort_order: i });
    }

    return ok({
        id: pollId,
        topic_id: topicId,
        question,
        max_choices: maxChoices,
        is_closed: false,
        options: createdOptions,
        total_votes: 0,
        user_votes: [],
        created_at: now
    });
};

Plugin.getPoll = (input) => {
    const topicId = routeParam(input, 1);
    let obj = input;
    if (typeof input === "string") { try { obj = JSON.parse(input); } catch (e) {} }
    const fullPath = obj.path || "";
    const qsIdx = fullPath.indexOf("?");
    let userId = "";
    if (qsIdx >= 0) {
        const qs = fullPath.substring(qsIdx + 1);
        const pairs = qs.split("&");
        for (let p = 0; p < pairs.length; p++) {
            if (pairs[p].indexOf("user_id=") === 0) {
                userId = decodeURIComponent(pairs[p].substring(8));
            }
        }
    }

    const polls = query("SELECT id, topic_id, question, max_choices, is_closed, created_at FROM forum_polls WHERE topic_id = ?", [topicId]);
    if (!polls || polls.length === 0) return ok(null);

    const poll = polls[0];
    const options = query("SELECT id, text, CAST(vote_count AS TEXT) as vote_count, CAST(sort_order AS TEXT) as sort_order FROM forum_poll_options WHERE poll_id = ? ORDER BY sort_order", [poll.id]);

    const totalResult = query("SELECT CAST(SUM(vote_count) AS TEXT) as total FROM forum_poll_options WHERE poll_id = ?", [poll.id]);
    const totalVotes = (totalResult?.[0]?.total) ? parseInt(totalResult[0].total, 10) : 0;

    const userVotes = [];
    if (userId) {
        const votes = query("SELECT option_id FROM forum_poll_votes WHERE poll_id = ? AND user_id = ?", [poll.id, userId]);
        if (votes) {
            for (const vote of votes) {
                userVotes.push(vote.option_id);
            }
        }
    }

    const fixedOptions = [];
    if (options) {
        for (const opt of options) {
            fixedOptions.push({
                id: opt.id,
                text: opt.text,
                vote_count: parseInt(opt.vote_count, 10) || 0,
                sort_order: parseInt(opt.sort_order, 10) || 0
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

Plugin.castVote = (input) => {
    const pollId = routeParam(input, 2);
    const data = parseBody(input);
    const userId = data.user_id;
    const optionIds = data.option_ids || [];

    if (!userId) return ok(err(400, "user_id required"));
    if (!optionIds || optionIds.length === 0) return ok(err(400, "option_ids required"));

    const polls = query("SELECT id, topic_id, question, max_choices, is_closed FROM forum_polls WHERE id = ?", [pollId]);
    if (!polls || polls.length === 0) return ok(err(404, "poll not found"));

    const poll = polls[0];
    if (parseInt(poll.is_closed, 10) === 1) return ok(err(400, "poll is closed"));

    const maxChoices = parseInt(poll.max_choices, 10);
    if (optionIds.length > maxChoices) return ok(err(400, `too many choices (max ${maxChoices})`));

    const existingVotes = query("SELECT option_id FROM forum_poll_votes WHERE poll_id = ? AND user_id = ?", [pollId, userId]);
    if (existingVotes?.length > 0) return ok(err(400, "already voted"));

    for (const optId of optionIds) {
        const opts = query("SELECT id FROM forum_poll_options WHERE id = ? AND poll_id = ?", [optId, pollId]);
        if (!opts || opts.length === 0) return ok(err(400, `option not found: ${optId}`));

        const voteId = genId();
        const now = nowISO();
        exec("INSERT INTO forum_poll_votes (id, tenant_id, poll_id, option_id, user_id, created_at, updated_at) VALUES (?, 'default', ?, ?, ?, ?, ?)",
            [voteId, pollId, optId, userId, now, now]);
        exec("UPDATE forum_poll_options SET vote_count = vote_count + 1 WHERE id = ?", [optId]);
    }

    return ok({ poll_id: pollId, voted_options: optionIds });
};

Plugin.deletePoll = (input) => {
    const pollId = routeParam(input, 1);
    const data = parseBody(input);
    const userId = data.user_id;

    if (!userId) return ok(err(400, "user_id required"));

    const polls = query("SELECT id, topic_id FROM forum_polls WHERE id = ?", [pollId]);
    if (!polls || polls.length === 0) return ok(err(404, "poll not found"));

    const topics = query("SELECT author_id FROM forum_topics WHERE id = ?", [polls[0].topic_id]);
    if (!topics || topics.length === 0 || topics[0].author_id !== userId) return ok(err(403, "only topic author can delete poll"));

    exec("DELETE FROM forum_poll_votes WHERE poll_id = ?", [pollId]);
    exec("DELETE FROM forum_poll_options WHERE poll_id = ?", [pollId]);
    exec("DELETE FROM forum_polls WHERE id = ?", [pollId]);

    return ok({ deleted: true });
};
