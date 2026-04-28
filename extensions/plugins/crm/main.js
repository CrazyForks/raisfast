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

const query = (sql, params) => {
    const result = Host.dbQuery(sql, params ? JSON.stringify(params) : null);
    if (!result || result.indexOf("error:") === 0) return null;
    return JSON.parse(result);
};

const exec = (sql, params) => {
    const result = Host.dbExecute(sql, params ? JSON.stringify(params) : null);
    return JSON.parse(result);
};

const genId = () => {
    return "xxxxxxxx-xxxx-4xxx-yxxx-xxxxxxxxxxxx".replace(/[xy]/g, (c) => {
        const r = (Math.random() * 16) | 0;
        const v = c === "x" ? r : (r & 0x3) | 0x8;
        return v.toString(16);
    });
};

const nowISO = () => new Date().toISOString();

const parseIntOrNull = (val) => (val != null ? parseInt(val, 10) : null);

// ── Hooks ───────────────────────────────────────────────────

Plugin.on_content_created = (input) => {
    const data = parseBody(input);
    const ct = data.content_type;

    if (ct === "contact") {
        Host.log("info", `[crm] new contact created: ${data.id}`);
        if (data.lifecycle_stage === "subscriber" || data.lifecycle_stage === "lead") {
            Host.emitEvent("crm.lead_created", JSON.stringify({
                contact_id: data.id,
                email: data.email,
                source: data.source,
            }));
        }
    }

    if (ct === "deal") {
        Host.log("info", `[crm] new deal created: ${data.id} stage=${data.stage}`);
        Host.emitEvent("crm.deal_created", JSON.stringify({
            deal_id: data.id,
            stage: data.stage,
            amount: data.amount,
        }));
    }

    if (ct === "activity") {
        Host.log("info", `[crm] new activity: ${data.id} type=${data.type}`);
    }

    return ok(data);
};

Plugin.on_content_updated = (input) => {
    const data = parseBody(input);
    const ct = data.content_type;

    if (ct === "deal") {
        Host.log("info", `[crm] deal updated: ${data.id}`);
        Host.emitEvent("crm.deal_updated", JSON.stringify({
            deal_id: data.id,
            stage: data.stage,
        }));
    }

    return ok(data);
};

// ── GET /pipeline ────────────────────────────────────────────

Plugin.getPipeline = (input) => {
    const stages = ["prospecting", "qualification", "proposal", "negotiation", "closed_won", "closed_lost"];
    const stageLabels = {
        prospecting: "初步接触",
        qualification: "需求确认",
        proposal: "方案报价",
        negotiation: "商务谈判",
        closed_won: "赢单",
        closed_lost: "丢单",
    };

    const pipeline = [];
    for (const stage of stages) {
        const rows = query(
            `SELECT id, title, amount, currency, probability, contact_id, company_id, owner_id, close_date
             FROM crm_deals WHERE stage = ? ORDER BY amount DESC`,
            [stage]
        );
        const totalResult = query(
            `SELECT CAST(COALESCE(SUM(amount), 0) AS TEXT) as total, CAST(COUNT(*) AS TEXT) as cnt
             FROM crm_deals WHERE stage = ?`,
            [stage]
        );
        const total = totalResult?.[0] ? parseFloat(totalResult[0].total) : 0;
        const count = totalResult?.[0] ? parseInt(totalResult[0].cnt, 10) : 0;
        const weighted = total * (stage === "closed_won" ? 100 : stage === "closed_lost" ? 0 : 30) / 100;

        pipeline.push({
            stage,
            label: stageLabels[stage],
            count,
            total_value: total,
            weighted_value: weighted,
            deals: rows || [],
        });
    }

    const totalPipeline = pipeline.reduce((sum, s) => sum + s.total_value, 0);
    const totalWeighted = pipeline.reduce((sum, s) => sum + s.weighted_value, 0);

    return ok({
        stages: pipeline,
        summary: {
            total_deals: pipeline.reduce((sum, s) => sum + s.count, 0),
            total_value: totalPipeline,
            weighted_value: totalWeighted,
        },
    });
};

// ── GET /pipeline/:dealId ────────────────────────────────────

Plugin.getDealDetail = (input) => {
    const dealId = routeParam(input, 2);
    if (!dealId) return ok(err(400, "deal id required"));

    const deals = query(
        `SELECT id, title, amount, currency, stage, probability,
                contact_id, company_id, owner_id, close_date, loss_reason, description,
                created_at, updated_at
         FROM crm_deals WHERE id = ?`,
        [dealId]
    );
    if (!deals || deals.length === 0) return ok(err(404, "deal not found"));

    const deal = deals[0];
    deal.amount = parseFloat(deal.amount) || 0;
    deal.probability = parseIntOrNull(deal.probability) || 0;

    const activities = query(
        `SELECT id, type, subject, activity_date, outcome, duration_minutes, owner_id, created_at
         FROM crm_activities WHERE deal_id = ? ORDER BY activity_date DESC LIMIT 20`,
        [dealId]
    );
    for (const a of (activities || [])) {
        a.duration_minutes = parseIntOrNull(a.duration_minutes);
    }

    const notes = query(
        `SELECT id, content, pinned, owner_id, created_at
         FROM crm_notes WHERE deal_id = ? ORDER BY pinned DESC, created_at DESC LIMIT 20`,
        [dealId]
    );

    deal.activities = activities || [];
    deal.notes = notes || [];

    return ok(deal);
};

// ── POST /deals/:dealId/stage ────────────────────────────────

Plugin.updateDealStage = (input) => {
    const dealId = routeParam(input, 2);
    const data = parseBody(input);
    const userId = data.user_id;
    const newStage = data.stage;
    const probability = data.probability;
    const lossReason = data.loss_reason;

    if (!userId) return ok(err(400, "user_id required"));
    if (!dealId) return ok(err(400, "deal id required"));
    if (!newStage) return ok(err(400, "stage required"));

    const validStages = ["prospecting", "qualification", "proposal", "negotiation", "closed_won", "closed_lost"];
    if (!validStages.includes(newStage)) return ok(err(400, `invalid stage: ${newStage}`));

    const deals = query("SELECT id, stage, title FROM crm_deals WHERE id = ?", [dealId]);
    if (!deals || deals.length === 0) return ok(err(404, "deal not found"));

    const oldStage = deals[0].stage;
    if (oldStage === newStage) return ok(err(400, "deal already in this stage"));

    const now = nowISO();
    const updates = ["stage = ?", "updated_at = ?"];
    const params = [newStage, now];

    if (probability != null) {
        updates.push("probability = ?");
        params.push(String(probability));
    }

    if (newStage === "closed_won") {
        updates.push("probability = ?");
        params.push("100");
    }

    if (newStage === "closed_lost") {
        updates.push("probability = ?");
        params.push("0");
        if (lossReason) {
            updates.push("loss_reason = ?");
            params.push(lossReason);
        }
    }

    params.push(dealId);
    const r = exec(`UPDATE crm_deals SET ${updates.join(", ")} WHERE id = ?`, params);
    if (r.error) return ok(err(500, r.error));

    const activityId = genId();
    const activityContent = JSON.stringify({ old_stage: oldStage, new_stage: newStage });
    exec(
        "INSERT INTO crm_activities (id, tenant_id, type, subject, content, deal_id, owner_id, activity_date, created_at, updated_at) VALUES (?, 'default', 'note', ?, ?, ?, ?, ?, ?, ?)",
        [activityId, `Stage: ${oldStage} → ${newStage}`, activityContent, dealId, userId, now, now, now]
    );

    Host.emitEvent("crm.deal_stage_changed", JSON.stringify({
        deal_id: dealId,
        deal_title: deals[0].title,
        old_stage: oldStage,
        new_stage: newStage,
        changed_by: userId,
    }));

    return ok({
        deal_id: dealId,
        old_stage: oldStage,
        new_stage: newStage,
        updated_at: now,
    });
};

// ── GET /contacts/:contactId/timeline ────────────────────────

Plugin.getContactTimeline = (input) => {
    const contactId = routeParam(input, 2);
    if (!contactId) return ok(err(400, "contact id required"));

    const contacts = query("SELECT id FROM crm_contacts WHERE id = ?", [contactId]);
    if (!contacts || contacts.length === 0) return ok(err(404, "contact not found"));

    const activities = query(
        `SELECT 'activity' as type, id, type as activity_type, subject, content, activity_date, outcome, duration_minutes, owner_id, created_at
         FROM crm_activities WHERE contact_id = ? ORDER BY activity_date DESC LIMIT 50`,
        [contactId]
    );

    const notes = query(
        `SELECT 'note' as type, id, content, pinned, owner_id, created_at
         FROM crm_notes WHERE contact_id = ? ORDER BY created_at DESC LIMIT 50`,
        [contactId]
    );

    const timeline = [];
    for (const a of (activities || [])) {
        a.duration_minutes = parseIntOrNull(a.duration_minutes);
        timeline.push({ ...a, timestamp: a.activity_date || a.created_at });
    }
    for (const n of (notes || [])) {
        timeline.push({ ...n, activity_type: "note", timestamp: n.created_at });
    }

    timeline.sort((a, b) => (b.timestamp || "").localeCompare(a.timestamp || ""));

    return ok({ contact_id: contactId, items: timeline.slice(0, 50) });
};

// ── GET /companies/:companyId/timeline ───────────────────────

Plugin.getCompanyTimeline = (input) => {
    const companyId = routeParam(input, 2);
    if (!companyId) return ok(err(400, "company id required"));

    const companies = query("SELECT id FROM crm_companies WHERE id = ?", [companyId]);
    if (!companies || companies.length === 0) return ok(err(404, "company not found"));

    const activities = query(
        `SELECT 'activity' as type, id, type as activity_type, subject, content, activity_date, outcome, owner_id, created_at
         FROM crm_activities WHERE company_id = ? ORDER BY activity_date DESC LIMIT 50`,
        [companyId]
    );

    const notes = query(
        `SELECT 'note' as type, id, content, pinned, owner_id, created_at
         FROM crm_notes WHERE company_id = ? ORDER BY created_at DESC LIMIT 50`,
        [companyId]
    );

    const deals = query(
        `SELECT 'deal' as type, id, title, stage, amount, currency, close_date, created_at
         FROM crm_deals WHERE company_id = ? ORDER BY created_at DESC LIMIT 20`,
        [companyId]
    );

    const timeline = [];
    for (const a of (activities || [])) {
        timeline.push({ ...a, timestamp: a.activity_date || a.created_at });
    }
    for (const n of (notes || [])) {
        timeline.push({ ...n, activity_type: "note", timestamp: n.created_at });
    }
    for (const d of (deals || [])) {
        d.amount = parseFloat(d.amount) || 0;
        timeline.push({ ...d, activity_type: "deal", timestamp: d.created_at });
    }

    timeline.sort((a, b) => (b.timestamp || "").localeCompare(a.timestamp || ""));

    return ok({ company_id: companyId, items: timeline.slice(0, 50) });
};

// ── GET /stats ───────────────────────────────────────────────

Plugin.getDashboardStats = () => {
    const contactCount = query("SELECT CAST(COUNT(*) AS TEXT) as cnt FROM crm_contacts");
    const totalContacts = contactCount?.[0] ? parseInt(contactCount[0].cnt, 10) : 0;

    const activeContacts = query("SELECT CAST(COUNT(*) AS TEXT) as cnt FROM crm_contacts WHERE status = 'active'");
    const activeCount = activeContacts?.[0] ? parseInt(activeContacts[0].cnt, 10) : 0;

    const companyCount = query("SELECT CAST(COUNT(*) AS TEXT) as cnt FROM crm_companies");
    const totalCompanies = companyCount?.[0] ? parseInt(companyCount[0].cnt, 10) : 0;

    const openDeals = query("SELECT CAST(COUNT(*) AS TEXT) as cnt, CAST(COALESCE(SUM(amount), 0) AS TEXT) as total FROM crm_deals WHERE stage NOT IN ('closed_won', 'closed_lost')");
    const openDealCount = openDeals?.[0] ? parseInt(openDeals[0].cnt, 10) : 0;
    const openDealValue = openDeals?.[0] ? parseFloat(openDeals[0].total) : 0;

    const wonDeals = query("SELECT CAST(COUNT(*) AS TEXT) as cnt, CAST(COALESCE(SUM(amount), 0) AS TEXT) as total FROM crm_deals WHERE stage = 'closed_won'");
    const wonDealCount = wonDeals?.[0] ? parseInt(wonDeals[0].cnt, 10) : 0;
    const wonDealValue = wonDeals?.[0] ? parseFloat(wonDeals[0].total) : 0;

    const lostDeals = query("SELECT CAST(COUNT(*) AS TEXT) as cnt FROM crm_deals WHERE stage = 'closed_lost'");
    const lostDealCount = lostDeals?.[0] ? parseInt(lostDeals[0].cnt, 10) : 0;

    const winRate = (wonDealCount + lostDealCount) > 0
        ? Math.round((wonDealCount / (wonDealCount + lostDealCount)) * 100)
        : 0;

    const avgDealSize = wonDealCount > 0 ? Math.round(wonDealValue / wonDealCount) : 0;

    const activityThisWeek = query(
        `SELECT CAST(COUNT(*) AS TEXT) as cnt FROM crm_activities WHERE activity_date >= date('now', '-7 days')`
    );
    const weeklyActivities = activityThisWeek?.[0] ? parseInt(activityThisWeek[0].cnt, 10) : 0;

    const contactsByStage = query(
        `SELECT lifecycle_stage, CAST(COUNT(*) AS TEXT) as cnt FROM crm_contacts GROUP BY lifecycle_stage ORDER BY cnt DESC`
    );
    const lifecycleDistribution = {};
    for (const row of (contactsByStage || [])) {
        lifecycleDistribution[row.lifecycle_stage] = parseInt(row.cnt, 10);
    }

    return ok({
        contacts: { total: totalContacts, active: activeCount },
        companies: { total: totalCompanies },
        deals: {
            open: openDealCount,
            open_value: openDealValue,
            won: wonDealCount,
            won_value: wonDealValue,
            lost: lostDealCount,
            win_rate: winRate,
            avg_deal_size: avgDealSize,
        },
        activities_this_week: weeklyActivities,
        lifecycle_distribution: lifecycleDistribution,
    });
};

// ── GET /leaderboard ─────────────────────────────────────────

Plugin.getLeaderboard = () => {
    const byDealValue = query(
        `SELECT owner_id, CAST(COUNT(*) AS TEXT) as deal_count, CAST(COALESCE(SUM(amount), 0) AS TEXT) as total_value
         FROM crm_deals WHERE stage = 'closed_won' GROUP BY owner_id ORDER BY total_value DESC LIMIT 10`
    );

    const byActivity = query(
        `SELECT owner_id, CAST(COUNT(*) AS TEXT) as activity_count
         FROM crm_activities WHERE activity_date >= date('now', '-30 days')
         GROUP BY owner_id ORDER BY activity_count DESC LIMIT 10`
    );

    const leaderboard = [];
    for (const row of (byDealValue || [])) {
        leaderboard.push({
            owner_id: row.owner_id,
            won_deals: parseInt(row.deal_count, 10),
            won_value: parseFloat(row.total_value),
        });
    }

    const activityLeaderboard = [];
    for (const row of (byActivity || [])) {
        activityLeaderboard.push({
            owner_id: row.owner_id,
            activities: parseInt(row.activity_count, 10),
        });
    }

    return ok({
        by_revenue: leaderboard,
        by_activity: activityLeaderboard,
    });
};

// ── POST /contacts/:contactId/convert ────────────────────────

Plugin.convertLead = (input) => {
    const contactId = routeParam(input, 2);
    const data = parseBody(input);
    const userId = data.user_id;
    if (!userId) return ok(err(400, "user_id required"));
    if (!contactId) return ok(err(400, "contact id required"));

    const contacts = query("SELECT id, lifecycle_stage, company_id FROM crm_contacts WHERE id = ?", [contactId]);
    if (!contacts || contacts.length === 0) return ok(err(404, "contact not found"));

    const contact = contacts[0];
    const currentStage = contact.lifecycle_stage;

    const stageOrder = ["subscriber", "lead", "marketing_qualified_lead", "sales_qualified_lead", "opportunity", "customer"];
    const currentIdx = stageOrder.indexOf(currentStage);
    if (currentIdx === -1) return ok(err(400, `unknown lifecycle stage: ${currentStage}`));

    const targetStage = data.target_stage || stageOrder[Math.min(currentIdx + 1, stageOrder.length - 1)];
    const targetIdx = stageOrder.indexOf(targetStage);
    if (targetIdx === -1) return ok(err(400, `invalid target stage: ${targetStage}`));
    if (targetIdx <= currentIdx) return ok(err(400, `cannot convert backward from ${currentStage} to ${targetStage}`));

    const now = nowISO();
    const r = exec(
        "UPDATE crm_contacts SET lifecycle_stage = ?, updated_at = ? WHERE id = ?",
        [targetStage, now, contactId]
    );
    if (r.error) return ok(err(500, r.error));

    const activityId = genId();
    exec(
        "INSERT INTO crm_activities (id, tenant_id, type, subject, content, contact_id, company_id, owner_id, activity_date, created_at, updated_at) VALUES (?, 'default', 'note', ?, ?, ?, ?, ?, ?, ?, ?)",
        [activityId, `Lifecycle: ${currentStage} → ${targetStage}`, JSON.stringify({ from: currentStage, to: targetStage }), contactId, contact.company_id || null, userId, now, now, now]
    );

    Host.emitEvent("crm.contact_converted", JSON.stringify({
        contact_id: contactId,
        from_stage: currentStage,
        to_stage: targetStage,
    }));

    return ok({
        contact_id: contactId,
        from_stage: currentStage,
        to_stage: targetStage,
        updated_at: now,
    });
};

// ── GET /reports/funnel ──────────────────────────────────────

Plugin.getFunnelReport = () => {
    const stages = ["prospecting", "qualification", "proposal", "negotiation", "closed_won", "closed_lost"];
    const stageLabels = {
        prospecting: "初步接触",
        qualification: "需求确认",
        proposal: "方案报价",
        negotiation: "商务谈判",
        closed_won: "赢单",
        closed_lost: "丢单",
    };

    const funnel = [];
    let prevCount = null;
    for (const stage of stages) {
        const result = query(
            `SELECT CAST(COUNT(*) AS TEXT) as cnt, CAST(COALESCE(SUM(amount), 0) AS TEXT) as total
             FROM crm_deals WHERE stage = ?`,
            [stage]
        );
        const count = result?.[0] ? parseInt(result[0].cnt, 10) : 0;
        const total = result?.[0] ? parseFloat(result[0].total) : 0;

        const conversionRate = prevCount != null && prevCount > 0
            ? Math.round((count / prevCount) * 100)
            : null;

        funnel.push({
            stage,
            label: stageLabels[stage],
            count,
            total_value: total,
            conversion_from_prev: conversionRate,
        });
        prevCount = count;
    }

    const totalOpen = funnel
        .filter(f => !["closed_won", "closed_lost"].includes(f.stage))
        .reduce((sum, f) => sum + f.count, 0);
    const overallConversion = totalOpen > 0 && funnel[4].count > 0
        ? Math.round((funnel[4].count / (funnel[4].count + funnel[5].count)) * 100)
        : 0;

    return ok({
        funnel,
        overall_win_rate: overallConversion,
        average_deal_cycle_days: null,
    });
};

// ── GET /reports/activities ──────────────────────────────────

Plugin.getActivityReport = () => {
    const byType = query(
        `SELECT type, CAST(COUNT(*) AS TEXT) as cnt FROM crm_activities GROUP BY type ORDER BY cnt DESC`
    );
    const typeBreakdown = {};
    for (const row of (byType || [])) {
        typeBreakdown[row.type] = parseInt(row.cnt, 10);
    }

    const byOwner = query(
        `SELECT owner_id, CAST(COUNT(*) AS TEXT) as cnt FROM crm_activities GROUP BY owner_id ORDER BY cnt DESC LIMIT 10`
    );
    const ownerBreakdown = [];
    for (const row of (byOwner || [])) {
        ownerBreakdown.push({ owner_id: row.owner_id, count: parseInt(row.cnt, 10) });
    }

    const last30 = query(
        `SELECT substr(activity_date, 1, 10) as day, CAST(COUNT(*) AS TEXT) as cnt
         FROM crm_activities
         WHERE activity_date >= date('now', '-30 days')
         GROUP BY day ORDER BY day`
    );
    const daily = [];
    for (const row of (last30 || [])) {
        daily.push({ date: row.day, count: parseInt(row.cnt, 10) });
    }

    return ok({
        by_type: typeBreakdown,
        by_owner: ownerBreakdown,
        daily_last_30_days: daily,
    });
};
