// chat.autoreply — the bot LLM step (architecture §4.1 step 5 / §4.2).
// Enqueued by chat.ingress (or delayed in fallback mode) with
// { trace_id, channel_key, conversation_id, bot_id }.
//
// Handoff triggers (all call `handoff`): visitor keyword, LLM failure /
// empty reply, first_line done. Coalesce: the job re-checks the conversation
// bot_status and aborts if an agent took over.

import {
    callApi,
    ctCreate,
    ctFind,
    ctGet,
    ctUpdate,
    getReceipt,
    logInfo,
} from 'sdk';
import {
    CT_BOT,
    CT_CONV,
    CT_MSG,
    idOf,
    parseJobInput,
} from '../lib/ctx.js';
import {
    emitAutoreplyFailed,
    emitIntegrationMessage,
    emitMessageCreated,
} from '../lib/events.js';

// Transfer to a human: fixed bot_status=disabled + status=open (architecture
// §4.4). After handoff the session stays human until an explicit toggle.
function handoff(convId, traceId, channelKey, reason) {
    ctUpdate(CT_CONV, convId, { bot_status: 'disabled', status: 'open' });
    emitAutoreplyFailed({
        trace_id: traceId,
        channel: channelKey,
        conversation_id: convId,
        reason,
    });
}

export function onAutoreply(input) {
    const job = parseJobInput(input);
    const { trace_id: traceId, channel_key: channelKey, conversation_id: convId, bot_id: botId } = job;
    if (!traceId || !channelKey || !convId || !botId) {
        throw new Error('chat.autoreply: payload must come from chat.ingress');
    }

    const bot = ctGet(CT_BOT, botId);
    if (!bot) throw new Error(`chat.autoreply: bot ${botId} not found`);
    const cfg = bot.autoreply;
    if (!cfg || !cfg.client) throw new Error('chat_bot.autoreply requires "client" (api-client key)');
    const mode = bot.mode ?? 'full';
    const contextWindow = Math.min(Math.max(cfg.context_window ?? 10, 1), 100);

    const conv = ctGet(CT_CONV, convId);
    if (!conv) throw new Error(`chat.autoreply: conversation ${convId} not found`);
    if (conv.bot_status !== 'active') {
        logInfo(`[chat] autoreply skipped (bot disabled) trace=${traceId}`);
        return { skipped: 'bot_disabled' };
    }
    if (mode === 'fallback' && conv.last_message_role === 'agent') {
        logInfo(`[chat] autoreply skipped (agent took over) trace=${traceId}`);
        return { skipped: 'agent_took_over' };
    }

    const receipt = getReceipt(traceId);
    const userText = receipt?.envelope?.payload?.body ?? '';

    // Visitor explicitly asked for a human → hand off without replying.
    const keywords = bot.handoff?.keywords ?? [];
    if (keywords.some((k) => k && userText.includes(k))) {
        handoff(convId, traceId, channelKey, 'visitor_requested');
        return { handoff: true };
    }

    // Context window (recent N messages, chronological).
    const res = ctFind(CT_MSG, {
        filters: [{ field: 'conversation_id', value: String(convId) }],
        sort: 'id desc',
        page_size: contextWindow,
    });
    const history = (res.rows ?? [])
        .slice()
        .reverse()
        .map((m) => ({ role: m.role ?? 'user', content: typeof m.body === 'string' ? m.body : String(m.body ?? '') }));
    if (userText && !history.some((m) => m.content === userText)) {
        history.push({ role: 'user', content: userText });
    }
    while (history.length > contextWindow) history.shift();

    // LLM call (openai | messages request styles).
    let llmInput;
    if (cfg.input_style === 'openai') {
        const messages = [];
        if (cfg.system_prompt) messages.push({ role: 'system', content: cfg.system_prompt });
        messages.push(...history);
        llmInput = cfg.model ? { model: cfg.model, messages } : { messages };
    } else {
        llmInput = { query: userText, messages: history };
        if (cfg.system_prompt) llmInput.system = cfg.system_prompt;
    }

    let replyText = '';
    try {
        const outRaw = callApi(cfg.client, cfg.op ?? 'chat', llmInput);
        const out = typeof outRaw === 'string' ? JSON.parse(outRaw) : outRaw;
        if (out && out.error) throw new Error(out.error);
        // Host envelope: {status, output, tokens_in, tokens_out, model}.
        let v = out.output ?? null;
        if (cfg.output_field) {
            for (const seg of String(cfg.output_field).split('.')) v = v?.[seg] ?? null;
        }
        replyText = typeof v === 'string' ? v : (v != null ? String(v) : '');
    } catch (e) {
        handoff(convId, traceId, channelKey, 'llm_failed');
        throw e;
    }
    if (!replyText) {
        handoff(convId, traceId, channelKey, 'empty_reply');
        throw new Error(`chat.autoreply: empty reply (trace ${traceId})`);
    }

    const assistant = ctCreate(CT_MSG, {
        conversation_id: String(convId),
        role: 'assistant',
        body: replyText,
        external_id: `reply-${traceId}`,
        receipt_id: traceId,
    });

    if (mode === 'first_line') {
        handoff(convId, traceId, channelKey, 'first_line_done');
    }

    emitIntegrationMessage({
        trace_id: traceId,
        channel: channelKey,
        conversation_id: convId,
        contact_id: conv.contact_id,
        message_id: idOf(assistant),
        role: 'assistant',
        body: replyText,
    });
    emitMessageCreated({
        trace_id: traceId,
        channel: channelKey,
        conversation_id: convId,
        contact_id: conv.contact_id,
        message_id: idOf(assistant),
        role: 'assistant',
        body: replyText,
    });
    logInfo(`[chat] autoreply delivered trace=${traceId} conv=${convId}`);
    return { conversation_id: convId };
}
