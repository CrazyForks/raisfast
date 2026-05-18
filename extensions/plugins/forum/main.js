import { dbQuery, dbExec, ok, fail, extractJson, logInfo, logWarn, logError, newId, eventEmit,
         dbInsert, dbFetchOne, dbFetchAll, dbUpdate, dbDelete, dbCount,
         dbIncrement, dbSum } from 'sdk';

const nowISO = () => new Date().toISOString();

// ── Hooks ───────────────────────────────────────────────────

export function on_content_creating(input) {
    const data = extractJson(input, "body");
    const ct = data.content_type;
    const body = data.data || {};

    if (ct === "forum_reply") {
        const topicId = body.topic_id;
        if (topicId) {
            const topic = dbFetchOne("forum_topics", { id: topicId });
            if (topic && topic.is_locked) {
                return fail(400, "topic is locked");
            }
        }
    }

    return ok(data);
}

export function on_content_created(input) {
    const data = extractJson(input, "body");
    const ct = data.content_type;
    const id = data.id;

    if (ct === "forum_topic") {
        const topic = dbFetchOne("forum_topics", { id });
        if (topic) {
            const now = nowISO();
            dbIncrement("forum_boards",
                { topic_count: 1, post_count: 1 },
                { id: topic.board_id },
                { set: { last_activity_at: now, last_topic_id: id } });
        }
    }

    if (ct === "forum_reply") {
        const reply = dbFetchOne("forum_replies", { id });
        if (reply) {
            const now = nowISO();
            dbIncrement("forum_topics",
                { reply_count: 1 },
                { id: reply.topic_id },
                { set: { last_reply_at: now, last_reply_user_id: reply.author_id, updated_at: now } });

            const topic = dbFetchOne("forum_topics", { id: reply.topic_id });
            if (topic) {
                dbIncrement("forum_boards",
                    { post_count: 1 },
                    { id: topic.board_id },
                    { set: { last_activity_at: now } });
            }
        }
    }

    return ok(data);
}

export function on_content_deleted(input) {
    const data = extractJson(input, "body");
    const ct = data.content_type;
    const id = data.id;

    if (ct === "forum_topic") {
        const topic = dbFetchOne("forum_topics", { id });
        if (topic) {
            dbIncrement("forum_boards",
                { topic_count: -1, post_count: -1 },
                { id: topic.board_id },
                { min: 0 });
        }
    }

    if (ct === "forum_reply") {
        const reply = dbFetchOne("forum_replies", { id });
        if (reply) {
            dbIncrement("forum_topics",
                { reply_count: -1 },
                { id: reply.topic_id },
                { min: 0 });
        }
    }

    return ok(data);
}

export function on_content_viewed(input) {
    const data = extractJson(input, "body");
    const ct = data.content_type;
    const id = data.id;

    if (ct === "forum_topic") {
        dbIncrement("forum_topics", { view_count: 1 }, { id });
    }

    return ok(data);
}

// ── GET /boards/:slug/topics ────────────────────────────────

export function listBoardTopics(input) {
    const slug = extractJson(input, "params.slug");
    let page = 1;
    let pageSize = 20;
    if (page < 1) page = 1;
    if (pageSize < 1 || pageSize > 100) pageSize = 20;
    const offset = (page - 1) * pageSize;

    const board = dbFetchOne("forum_boards", { slug });
    if (!board) return fail(404, `board not found for slug: ${slug}`);
    const boardId = board.id;

    const total = dbCount("forum_topics", { board_id: boardId });

    const result = dbFetchAll("forum_topics", { board_id: boardId },
        { order_by: "is_pinned DESC, last_reply_at DESC, created_at DESC", limit: pageSize, offset });

    return ok({ items: result.data || [], total, page, page_size: pageSize, board_id: boardId });
}

// ── PUT /replies/:id/accept ─────────────────────────────────

export function acceptAnswer(input) {
    const replyId = extractJson(input, "params.id");
    const data = extractJson(input, "body");
    const userId = data.user_id;
    if (!userId) return fail(400, "user_id required");

    const reply = dbFetchOne("forum_replies", { id: replyId });
    if (!reply) return fail(404, "reply not found");

    const topic = dbFetchOne("forum_topics", { id: reply.topic_id });
    if (!topic) return fail(404, "topic not found");

    if (topic.author_id !== userId) return fail(403, "only topic author can accept answer");

    const now = nowISO();
    dbUpdate("forum_replies", { is_answer: 0 }, { topic_id: reply.topic_id, is_answer: 1 });
    dbUpdate("forum_replies", { is_answer: 1, updated_at: now }, { id: replyId });
    dbUpdate("forum_topics", { is_solved: 1, updated_at: now }, { id: reply.topic_id });

    return ok({ id: replyId, is_answer: true, topic_id: reply.topic_id });
}

// ── POST /vote ──────────────────────────────────────────────

export function vote(input) {
    const data = extractJson(input, "body");
    let userId = data.user_id;
    const targetType = data.target_type;
    const targetId = data.target_id;
    let value = data.value || 1;
    if (value < -1) value = -1;
    if (value > 1) value = 1;

    if (!userId) return fail(400, "user_id required");
    if (!targetType) return fail(400, "target_type required");
    if (!targetId) return fail(400, "target_id required");
    if (targetType !== "topic" && targetType !== "reply") return fail(400, "target_type must be topic or reply");

    const existing = dbFetchOne("forum_votes", { target_type: targetType, target_id: targetId, user_id: userId });
    if (existing) {
        const oldValue = parseInt(existing.value, 10);
        const diff = value - oldValue;
        if (diff === 0) return fail(400, "already voted");
        dbUpdate("forum_votes", { value, updated_at: nowISO() }, { id: existing.id });
        updateVoteCount(targetType, targetId, diff);
    } else {
        const id = newId();
        const now = nowISO();
        dbInsert("forum_votes", { id, tenant_id: "default", target_type: targetType, target_id: targetId, user_id: userId, value, created_at: now, updated_at: now });
        updateVoteCount(targetType, targetId, value);
    }

    return ok({ target_type: targetType, target_id: targetId, value });
}

// ── DELETE /vote ────────────────────────────────────────────

export function unvote(input) {
    const data = extractJson(input, "body");
    const userId = data.user_id;
    const targetType = data.target_type;
    const targetId = data.target_id;

    if (!userId) return fail(400, "user_id required");
    if (!targetType) return fail(400, "target_type required");
    if (!targetId) return fail(400, "target_id required");

    const existing = dbFetchOne("forum_votes", { target_type: targetType, target_id: targetId, user_id: userId });
    if (!existing) return fail(404, "vote not found");

    const oldValue = parseInt(existing.value, 10);
    dbDelete("forum_votes", { id: existing.id });
    updateVoteCount(targetType, targetId, -oldValue);

    return ok({ removed: true });
}

const updateVoteCount = (targetType, targetId, diff) => {
    const table = targetType === "topic" ? "forum_topics" : "forum_replies";
    dbIncrement(table, { vote_count: diff }, { id: targetId });
};

// ── Polls ───────────────────────────────────────────────────

export function createPoll(input) {
    const data = extractJson(input, "body");
    const userId = data.user_id;
    const topicId = data.topic_id;
    const question = (data.question || "").trim();
    const options = data.options || [];
    let maxChoices = data.max_choices || 1;

    if (!userId) return fail(400, "user_id required");
    if (!topicId) return fail(400, "topic_id required");
    if (!question) return fail(400, "question required");
    if (!options || options.length < 2) return fail(400, "at least 2 options required");
    if (options.length > 20) return fail(400, "too many options (max 20)");
    if (maxChoices < 1) maxChoices = 1;
    if (maxChoices > options.length) maxChoices = options.length;

    const topic = dbFetchOne("forum_topics", { id: topicId });
    if (!topic) return fail(404, "topic not found");
    if (topic.author_id !== userId) return fail(403, "only topic author can create poll");

    const existing = dbFetchOne("forum_polls", { topic_id: topicId });
    if (existing) return fail(400, "poll already exists for this topic");

    const pollId = newId();
    const now = nowISO();
    dbInsert("forum_polls", { id: pollId, tenant_id: "default", topic_id: topicId, question, max_choices: maxChoices, is_closed: 0, created_at: now, updated_at: now });

    const createdOptions = [];
    for (let i = 0; i < options.length; i++) {
        const optText = (options[i] || "").trim();
        if (!optText) continue;
        const optId = newId();
        dbInsert("forum_poll_options", { id: optId, tenant_id: "default", poll_id: pollId, text: optText, vote_count: 0, sort_order: i });
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
}

export function getPoll(input) {
    const topicId = extractJson(input, "params.topicId");
    let obj = input;
    if (typeof input === "string") { try { obj = JSON.parse(input); } catch {} }
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

    const poll = dbFetchOne("forum_polls", { topic_id: topicId });
    if (!poll) return ok(null);

    const optsResult = dbFetchAll("forum_poll_options", { poll_id: poll.id }, { order_by: "sort_order ASC" });
    const options = optsResult.data || [];

    const totalVotes = dbSum("forum_poll_options", "vote_count", { poll_id: poll.id });

    const userVotes = [];
    if (userId) {
        const votesResult = dbFetchAll("forum_poll_votes", { poll_id: poll.id, user_id: userId });
        if (votesResult.data) {
            for (const v of votesResult.data) {
                userVotes.push(v.option_id);
            }
        }
    }

    const fixedOptions = [];
    for (const opt of options) {
        fixedOptions.push({
            id: opt.id,
            text: opt.text,
            vote_count: parseInt(opt.vote_count, 10) || 0,
            sort_order: parseInt(opt.sort_order, 10) || 0
        });
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
}

export function castVote(input) {
    const pollId = extractJson(input, "params.pollId");
    const data = extractJson(input, "body");
    const userId = data.user_id;
    const optionIds = data.option_ids || [];

    if (!userId) return fail(400, "user_id required");
    if (!optionIds || optionIds.length === 0) return fail(400, "option_ids required");

    const poll = dbFetchOne("forum_polls", { id: pollId });
    if (!poll) return fail(404, "poll not found");

    if (parseInt(poll.is_closed, 10) === 1) return fail(400, "poll is closed");

    const maxChoices = parseInt(poll.max_choices, 10);
    if (optionIds.length > maxChoices) return fail(400, `too many choices (max ${maxChoices})`);

    const existingVotes = dbFetchAll("forum_poll_votes", { poll_id: pollId, user_id: userId });
    if (existingVotes.data && existingVotes.data.length > 0) return fail(400, "already voted");

    for (const optId of optionIds) {
        const opt = dbFetchOne("forum_poll_options", { id: optId, poll_id: pollId });
        if (!opt) return fail(400, `option not found: ${optId}`);

        const voteId = newId();
        const now = nowISO();
        dbInsert("forum_poll_votes", { id: voteId, tenant_id: "default", poll_id: pollId, option_id: optId, user_id: userId, created_at: now, updated_at: now });
        dbIncrement("forum_poll_options", { vote_count: 1 }, { id: optId });
    }

    return ok({ poll_id: pollId, voted_options: optionIds });
}

export function deletePoll(input) {
    const pollId = extractJson(input, "params.pollId");
    const data = extractJson(input, "body");
    const userId = data.user_id;

    if (!userId) return fail(400, "user_id required");

    const poll = dbFetchOne("forum_polls", { id: pollId });
    if (!poll) return fail(404, "poll not found");

    const topic = dbFetchOne("forum_topics", { id: poll.topic_id });
    if (!topic || topic.author_id !== userId) return fail(403, "only topic author can delete poll");

    dbDelete("forum_poll_votes", { poll_id: pollId });
    dbDelete("forum_poll_options", { poll_id: pollId });
    dbDelete("forum_polls", { id: pollId });

    return ok({ deleted: true });
}
