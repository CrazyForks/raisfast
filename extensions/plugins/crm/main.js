import { dbQuery, dbExec, ok, fail, extractJson, logInfo, eventEmit, newId,
         dbInsert, dbFetchOne, dbFetchAll, dbUpdate, dbDelete, dbCount,
         dbIncrement, dbSum, dbGroupBy } from 'sdk';

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
        const rows = dbFetchAll("crm_deals", { stage }, { order_by: "amount DESC" }).data || [];
        const total = dbSum("crm_deals", "amount", { stage });
        const count = dbCount("crm_deals", { stage });
        const weighted = total * (stage === "closed_won" ? 100 : stage === "closed_lost" ? 0 : 30) / 100;

        pipeline.push({
            stage,
            label: stageLabels[stage],
            count,
            total_value: total,
            weighted_value: weighted,
            deals: rows,
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

    const deal = dbFetchOne("crm_deals", { id: dealId });
    if (!deal) return fail(404, "deal not found");

    deal.amount = parseFloat(deal.amount) || 0;
    deal.probability = parseIntOrNull(deal.probability) || 0;

    const activities = dbFetchAll("crm_activities", { deal_id: dealId }, { order_by: "activity_date DESC", limit: 20 }).data || [];
    for (const a of activities) {
        a.duration_minutes = parseIntOrNull(a.duration_minutes);
    }

    const notes = dbFetchAll("crm_notes", { deal_id: dealId }, { order_by: "pinned DESC, created_at DESC", limit: 20 }).data || [];

    deal.activities = activities;
    deal.notes = notes;

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

    const deal = dbFetchOne("crm_deals", { id: dealId });
    if (!deal) return fail(404, "deal not found");

    const oldStage = deal.stage;
    if (oldStage === newStage) return fail(400, "deal already in this stage");

    const now = nowISO();
    const setData = { stage: newStage, updated_at: now };

    if (probability != null) {
        setData.probability = String(probability);
    }

    if (newStage === "closed_won") {
        setData.probability = "100";
    }

    if (newStage === "closed_lost") {
        setData.probability = "0";
        if (lossReason) {
            setData.loss_reason = lossReason;
        }
    }

    const r = dbUpdate("crm_deals", setData, { id: dealId });
    if (r.error) return fail(500, r.error);

    const activityId = newId();
    const activityContent = JSON.stringify({ old_stage: oldStage, new_stage: newStage });
    dbInsert("crm_activities", { id: activityId, tenant_id: "default", type: "note", subject: `Stage: ${oldStage} → ${newStage}`, content: activityContent, deal_id: dealId, owner_id: userId, activity_date: now, created_at: now, updated_at: now });

    eventEmit("crm.deal_stage_changed", JSON.stringify({
        deal_id: dealId,
        deal_title: deal.title,
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

    const contact = dbFetchOne("crm_contacts", { id: contactId });
    if (!contact) return fail(404, "contact not found");

    const rawActivities = dbFetchAll("crm_activities", { contact_id: contactId }, { order_by: "activity_date DESC", limit: 50 }).data || [];
    const rawNotes = dbFetchAll("crm_notes", { contact_id: contactId }, { order_by: "created_at DESC", limit: 50 }).data || [];

    const timeline = [];
    for (const a of rawActivities) {
        a.duration_minutes = parseIntOrNull(a.duration_minutes);
        timeline.push({ ...a, activity_type: a.type, type: 'activity', timestamp: a.activity_date || a.created_at });
    }
    for (const n of rawNotes) {
        timeline.push({ ...n, activity_type: "note", type: 'note', timestamp: n.created_at });
    }

    timeline.sort((a, b) => (b.timestamp || "").localeCompare(a.timestamp || ""));

    return ok({ contact_id: contactId, items: timeline.slice(0, 50) });
}

// ── GET /companies/:companyId/timeline ───────────────────────

export function getCompanyTimeline(input) {
    const companyId = extractJson(input, "params.companyId");
    if (!companyId) return fail(400, "company id required");

    const company = dbFetchOne("crm_companies", { id: companyId });
    if (!company) return fail(404, "company not found");

    const rawActivities = dbFetchAll("crm_activities", { company_id: companyId }, { order_by: "activity_date DESC", limit: 50 }).data || [];
    const rawNotes = dbFetchAll("crm_notes", { company_id: companyId }, { order_by: "created_at DESC", limit: 50 }).data || [];
    const rawDeals = dbFetchAll("crm_deals", { company_id: companyId }, { order_by: "created_at DESC", limit: 20 }).data || [];

    const timeline = [];
    for (const a of rawActivities) {
        timeline.push({ ...a, activity_type: a.type, type: 'activity', timestamp: a.activity_date || a.created_at });
    }
    for (const n of rawNotes) {
        timeline.push({ ...n, activity_type: "note", type: 'note', timestamp: n.created_at });
    }
    for (const d of rawDeals) {
        d.amount = parseFloat(d.amount) || 0;
        timeline.push({ ...d, activity_type: "deal", type: 'deal', timestamp: d.created_at });
    }

    timeline.sort((a, b) => (b.timestamp || "").localeCompare(a.timestamp || ""));

    return ok({ company_id: companyId, items: timeline.slice(0, 50) });
}

// ── GET /stats ───────────────────────────────────────────────

export function getDashboardStats() {
    const totalContacts = dbCount("crm_contacts");
    const activeCount = dbCount("crm_contacts", { status: "active" });
    const totalCompanies = dbCount("crm_companies");

    const openDealCount = dbCount("crm_deals", "stage NOT IN ('closed_won', 'closed_lost')");
    const openDealValue = dbSum("crm_deals", "amount", "stage NOT IN ('closed_won', 'closed_lost')");

    const wonDealCount = dbCount("crm_deals", { stage: "closed_won" });
    const wonDealValue = dbSum("crm_deals", "amount", { stage: "closed_won" });

    const lostDealCount = dbCount("crm_deals", { stage: "closed_lost" });

    const winRate = (wonDealCount + lostDealCount) > 0
        ? Math.round((wonDealCount / (wonDealCount + lostDealCount)) * 100)
        : 0;

    const avgDealSize = wonDealCount > 0 ? Math.round(wonDealValue / wonDealCount) : 0;

    const activityThisWeek = dbQuery(
        `SELECT CAST(COUNT(*) AS TEXT) as cnt FROM crm_activities WHERE activity_date >= date('now', '-7 days')`
    );
    const weeklyActivities = activityThisWeek?.[0] ? parseInt(activityThisWeek[0].cnt, 10) : 0;

    const lifecycleResult = dbGroupBy("crm_contacts", {
        group_by: "lifecycle_stage",
        count: true,
        order_by: "cnt DESC"
    });
    const lifecycleDistribution = {};
    for (const row of (lifecycleResult.data || [])) {
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
    const dealResult = dbGroupBy("crm_deals", {
        group_by: "owner_id",
        count: true,
        sum: "amount",
        where: { stage: "closed_won" },
        order_by: "sum_amount DESC",
        limit: 10
    });

    const activityResult = dbGroupBy("crm_activities", {
        group_by: "owner_id",
        count: true,
        where: "activity_date >= date('now', '-30 days')",
        order_by: "cnt DESC",
        limit: 10
    });

    const leaderboard = [];
    for (const row of (dealResult.data || [])) {
        leaderboard.push({
            owner_id: row.owner_id,
            won_deals: parseInt(row.cnt, 10),
            won_value: parseFloat(row.sum_amount),
        });
    }

    const activityLeaderboard = [];
    for (const row of (activityResult.data || [])) {
        activityLeaderboard.push({
            owner_id: row.owner_id,
            activities: parseInt(row.cnt, 10),
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

    const contact = dbFetchOne("crm_contacts", { id: contactId });
    if (!contact) return fail(404, "contact not found");

    const currentStage = contact.lifecycle_stage;

    const stageOrder = ["subscriber", "lead", "marketing_qualified_lead", "sales_qualified_lead", "opportunity", "customer"];
    const currentIdx = stageOrder.indexOf(currentStage);
    if (currentIdx === -1) return fail(400, `unknown lifecycle stage: ${currentStage}`);

    const targetStage = data.target_stage || stageOrder[Math.min(currentIdx + 1, stageOrder.length - 1)];
    const targetIdx = stageOrder.indexOf(targetStage);
    if (targetIdx === -1) return fail(400, `invalid target stage: ${targetStage}`);
    if (targetIdx <= currentIdx) return fail(400, `cannot convert backward from ${currentStage} to ${targetStage}`);

    const now = nowISO();
    const r = dbUpdate("crm_contacts", { lifecycle_stage: targetStage, updated_at: now }, { id: contactId });
    if (r.error) return fail(500, r.error);

    const activityId = newId();
    dbInsert("crm_activities", { id: activityId, tenant_id: "default", type: "note", subject: `Lifecycle: ${currentStage} → ${targetStage}`, content: JSON.stringify({ from: currentStage, to: targetStage }), contact_id: contactId, company_id: contact.company_id || null, owner_id: userId, activity_date: now, created_at: now, updated_at: now });

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
        const count = dbCount("crm_deals", { stage });
        const total = dbSum("crm_deals", "amount", { stage });

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
    const typeResult = dbGroupBy("crm_activities", {
        group_by: "type",
        count: true,
        order_by: "cnt DESC"
    });
    const typeBreakdown = {};
    for (const row of (typeResult.data || [])) {
        typeBreakdown[row.type] = parseInt(row.cnt, 10);
    }

    const ownerResult = dbGroupBy("crm_activities", {
        group_by: "owner_id",
        count: true,
        order_by: "cnt DESC",
        limit: 10
    });
    const ownerBreakdown = [];
    for (const row of (ownerResult.data || [])) {
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
