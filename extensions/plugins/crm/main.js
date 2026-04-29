import { dbQuery, dbExec, ok, fail, extractJson, logInfo, eventEmit, newId } from 'sdk';

const nowISO = () => new Date().toISOString();

const parseIntOrNull = (val) => (val != null ? parseInt(val, 10) : null);

// ── Hooks ───────────────────────────────────────────────────

export function on_content_created(input) {
    const data = extractJson(input, "body");
    const ct = data.content_type;

    if (ct === "contact") {
        logInfo(`[crm] new contact created: ${data.id}`);
        if (data.lifecycle_stage === "subscriber" || data.lifecycle_stage === "lead") {
            eventEmit("crm.lead_created", JSON.stringify({
                contact_id: data.id,
                email: data.email,
                source: data.source,
            }));
        }
    }

    if (ct === "deal") {
        logInfo(`[crm] new deal created: ${data.id} stage=${data.stage}`);
        eventEmit("crm.deal_created", JSON.stringify({
            deal_id: data.id,
            stage: data.stage,
            amount: data.amount,
        }));
    }

    if (ct === "activity") {
        logInfo(`[crm] new activity: ${data.id} type=${data.type}`);
    }

    return ok(data);
}

export function on_content_updated(input) {
    const data = extractJson(input, "body");
    const ct = data.content_type;

    if (ct === "deal") {
        logInfo(`[crm] deal updated: ${data.id}`);
        eventEmit("crm.deal_updated", JSON.stringify({
            deal_id: data.id,
            stage: data.stage,
        }));
    }

    return ok(data);
}

// ── GET /pipeline ────────────────────────────────────────────

export function getPipeline(input) {
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
        const rows = dbQuery(
            `SELECT id, title, amount, currency, probability, contact_id, company_id, owner_id, close_date
             FROM crm_deals WHERE stage = ? ORDER BY amount DESC`,
            [stage]
        );
        const totalResult = dbQuery(
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
}

// ── GET /pipeline/:dealId ────────────────────────────────────

export function getDealDetail(input) {
    const dealId = extractJson(input, "params.dealId");
    if (!dealId) return fail(400, "deal id required");

    const deals = dbQuery(
        `SELECT id, title, amount, currency, stage, probability,
                contact_id, company_id, owner_id, close_date, loss_reason, description,
                created_at, updated_at
         FROM crm_deals WHERE id = ?`,
        [dealId]
    );
    if (!deals || deals.length === 0) return fail(404, "deal not found");

    const deal = deals[0];
    deal.amount = parseFloat(deal.amount) || 0;
    deal.probability = parseIntOrNull(deal.probability) || 0;

    const activities = dbQuery(
        `SELECT id, type, subject, activity_date, outcome, duration_minutes, owner_id, created_at
         FROM crm_activities WHERE deal_id = ? ORDER BY activity_date DESC LIMIT 20`,
        [dealId]
    );
    for (const a of (activities || [])) {
        a.duration_minutes = parseIntOrNull(a.duration_minutes);
    }

    const notes = dbQuery(
        `SELECT id, content, pinned, owner_id, created_at
         FROM crm_notes WHERE deal_id = ? ORDER BY pinned DESC, created_at DESC LIMIT 20`,
        [dealId]
    );

    deal.activities = activities || [];
    deal.notes = notes || [];

    return ok(deal);
}

// ── POST /deals/:dealId/stage ────────────────────────────────

export function updateDealStage(input) {
    const dealId = extractJson(input, "params.dealId");
    const data = extractJson(input, "body");
    const userId = data.user_id;
    const newStage = data.stage;
    const probability = data.probability;
    const lossReason = data.loss_reason;

    if (!userId) return fail(400, "user_id required");
    if (!dealId) return fail(400, "deal id required");
    if (!newStage) return fail(400, "stage required");

    const validStages = ["prospecting", "qualification", "proposal", "negotiation", "closed_won", "closed_lost"];
    if (!validStages.includes(newStage)) return fail(400, `invalid stage: ${newStage}`);

    const deals = dbQuery("SELECT id, stage, title FROM crm_deals WHERE id = ?", [dealId]);
    if (!deals || deals.length === 0) return fail(404, "deal not found");

    const oldStage = deals[0].stage;
    if (oldStage === newStage) return fail(400, "deal already in this stage");

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
    const r = dbExec(`UPDATE crm_deals SET ${updates.join(", ")} WHERE id = ?`, params);
    if (r.error) return fail(500, r.error);

    const activityId = newId();
    const activityContent = JSON.stringify({ old_stage: oldStage, new_stage: newStage });
    dbExec(
        "INSERT INTO crm_activities (id, tenant_id, type, subject, content, deal_id, owner_id, activity_date, created_at, updated_at) VALUES (?, 'default', 'note', ?, ?, ?, ?, ?, ?, ?)",
        [activityId, `Stage: ${oldStage} → ${newStage}`, activityContent, dealId, userId, now, now, now]
    );

    eventEmit("crm.deal_stage_changed", JSON.stringify({
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
}

// ── GET /contacts/:contactId/timeline ────────────────────────

export function getContactTimeline(input) {
    const contactId = extractJson(input, "params.contactId");
    if (!contactId) return fail(400, "contact id required");

    const contacts = dbQuery("SELECT id FROM crm_contacts WHERE id = ?", [contactId]);
    if (!contacts || contacts.length === 0) return fail(404, "contact not found");

    const activities = dbQuery(
        `SELECT 'activity' as type, id, type as activity_type, subject, content, activity_date, outcome, duration_minutes, owner_id, created_at
         FROM crm_activities WHERE contact_id = ? ORDER BY activity_date DESC LIMIT 50`,
        [contactId]
    );

    const notes = dbQuery(
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
}

// ── GET /companies/:companyId/timeline ───────────────────────

export function getCompanyTimeline(input) {
    const companyId = extractJson(input, "params.companyId");
    if (!companyId) return fail(400, "company id required");

    const companies = dbQuery("SELECT id FROM crm_companies WHERE id = ?", [companyId]);
    if (!companies || companies.length === 0) return fail(404, "company not found");

    const activities = dbQuery(
        `SELECT 'activity' as type, id, type as activity_type, subject, content, activity_date, outcome, owner_id, created_at
         FROM crm_activities WHERE company_id = ? ORDER BY activity_date DESC LIMIT 50`,
        [companyId]
    );

    const notes = dbQuery(
        `SELECT 'note' as type, id, content, pinned, owner_id, created_at
         FROM crm_notes WHERE company_id = ? ORDER BY created_at DESC LIMIT 50`,
        [companyId]
    );

    const deals = dbQuery(
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
}

// ── GET /stats ───────────────────────────────────────────────

export function getDashboardStats() {
    const contactCount = dbQuery("SELECT CAST(COUNT(*) AS TEXT) as cnt FROM crm_contacts");
    const totalContacts = contactCount?.[0] ? parseInt(contactCount[0].cnt, 10) : 0;

    const activeContacts = dbQuery("SELECT CAST(COUNT(*) AS TEXT) as cnt FROM crm_contacts WHERE status = 'active'");
    const activeCount = activeContacts?.[0] ? parseInt(activeContacts[0].cnt, 10) : 0;

    const companyCount = dbQuery("SELECT CAST(COUNT(*) AS TEXT) as cnt FROM crm_companies");
    const totalCompanies = companyCount?.[0] ? parseInt(companyCount[0].cnt, 10) : 0;

    const openDeals = dbQuery("SELECT CAST(COUNT(*) AS TEXT) as cnt, CAST(COALESCE(SUM(amount), 0) AS TEXT) as total FROM crm_deals WHERE stage NOT IN ('closed_won', 'closed_lost')");
    const openDealCount = openDeals?.[0] ? parseInt(openDeals[0].cnt, 10) : 0;
    const openDealValue = openDeals?.[0] ? parseFloat(openDeals[0].total) : 0;

    const wonDeals = dbQuery("SELECT CAST(COUNT(*) AS TEXT) as cnt, CAST(COALESCE(SUM(amount), 0) AS TEXT) as total FROM crm_deals WHERE stage = 'closed_won'");
    const wonDealCount = wonDeals?.[0] ? parseInt(wonDeals[0].cnt, 10) : 0;
    const wonDealValue = wonDeals?.[0] ? parseFloat(wonDeals[0].total) : 0;

    const lostDeals = dbQuery("SELECT CAST(COUNT(*) AS TEXT) as cnt FROM crm_deals WHERE stage = 'closed_lost'");
    const lostDealCount = lostDeals?.[0] ? parseInt(lostDeals[0].cnt, 10) : 0;

    const winRate = (wonDealCount + lostDealCount) > 0
        ? Math.round((wonDealCount / (wonDealCount + lostDealCount)) * 100)
        : 0;

    const avgDealSize = wonDealCount > 0 ? Math.round(wonDealValue / wonDealCount) : 0;

    const activityThisWeek = dbQuery(
        `SELECT CAST(COUNT(*) AS TEXT) as cnt FROM crm_activities WHERE activity_date >= date('now', '-7 days')`
    );
    const weeklyActivities = activityThisWeek?.[0] ? parseInt(activityThisWeek[0].cnt, 10) : 0;

    const contactsByStage = dbQuery(
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
}

// ── GET /leaderboard ─────────────────────────────────────────

export function getLeaderboard() {
    const byDealValue = dbQuery(
        `SELECT owner_id, CAST(COUNT(*) AS TEXT) as deal_count, CAST(COALESCE(SUM(amount), 0) AS TEXT) as total_value
         FROM crm_deals WHERE stage = 'closed_won' GROUP BY owner_id ORDER BY total_value DESC LIMIT 10`
    );

    const byActivity = dbQuery(
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
}

// ── POST /contacts/:contactId/convert ────────────────────────

export function convertLead(input) {
    const contactId = extractJson(input, "params.contactId");
    const data = extractJson(input, "body");
    const userId = data.user_id;
    if (!userId) return fail(400, "user_id required");
    if (!contactId) return fail(400, "contact id required");

    const contacts = dbQuery("SELECT id, lifecycle_stage, company_id FROM crm_contacts WHERE id = ?", [contactId]);
    if (!contacts || contacts.length === 0) return fail(404, "contact not found");

    const contact = contacts[0];
    const currentStage = contact.lifecycle_stage;

    const stageOrder = ["subscriber", "lead", "marketing_qualified_lead", "sales_qualified_lead", "opportunity", "customer"];
    const currentIdx = stageOrder.indexOf(currentStage);
    if (currentIdx === -1) return fail(400, `unknown lifecycle stage: ${currentStage}`);

    const targetStage = data.target_stage || stageOrder[Math.min(currentIdx + 1, stageOrder.length - 1)];
    const targetIdx = stageOrder.indexOf(targetStage);
    if (targetIdx === -1) return fail(400, `invalid target stage: ${targetStage}`);
    if (targetIdx <= currentIdx) return fail(400, `cannot convert backward from ${currentStage} to ${targetStage}`);

    const now = nowISO();
    const r = dbExec(
        "UPDATE crm_contacts SET lifecycle_stage = ?, updated_at = ? WHERE id = ?",
        [targetStage, now, contactId]
    );
    if (r.error) return fail(500, r.error);

    const activityId = newId();
    dbExec(
        "INSERT INTO crm_activities (id, tenant_id, type, subject, content, contact_id, company_id, owner_id, activity_date, created_at, updated_at) VALUES (?, 'default', 'note', ?, ?, ?, ?, ?, ?, ?, ?)",
        [activityId, `Lifecycle: ${currentStage} → ${targetStage}`, JSON.stringify({ from: currentStage, to: targetStage }), contactId, contact.company_id || null, userId, now, now, now]
    );

    eventEmit("crm.contact_converted", JSON.stringify({
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
}

// ── GET /reports/funnel ──────────────────────────────────────

export function getFunnelReport() {
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
        const result = dbQuery(
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
}

// ── GET /reports/activities ──────────────────────────────────

export function getActivityReport() {
    const byType = dbQuery(
        `SELECT type, CAST(COUNT(*) AS TEXT) as cnt FROM crm_activities GROUP BY type ORDER BY cnt DESC`
    );
    const typeBreakdown = {};
    for (const row of (byType || [])) {
        typeBreakdown[row.type] = parseInt(row.cnt, 10);
    }

    const byOwner = dbQuery(
        `SELECT owner_id, CAST(COUNT(*) AS TEXT) as cnt FROM crm_activities GROUP BY owner_id ORDER BY cnt DESC LIMIT 10`
    );
    const ownerBreakdown = [];
    for (const row of (byOwner || [])) {
        ownerBreakdown.push({ owner_id: row.owner_id, count: parseInt(row.cnt, 10) });
    }

    const last30 = dbQuery(
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
}
