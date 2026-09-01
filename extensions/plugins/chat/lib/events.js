// Chat event emission wrappers. Keeps event payload contracts in one place
// so both the ingress/autoreply/egress jobs and the workspace/widget routes
// emit identical shapes.

import { eventEmit, logInfo } from 'sdk';

export function emitMessageCreated(payload) {
    eventEmit('chat.message.created', payload);
    logInfo(`[chat] message.created conv=${payload.conversation_id} role=${payload.role}`);
}

export function emitConversationUpdated(payload) {
    eventEmit('chat.conversation.updated', payload);
}

export function emitAssignment(payload) {
    eventEmit('chat.assignment', payload);
}

export function emitBotToggled(payload) {
    eventEmit('chat.bot.toggled', payload);
}

export function emitTyping(payload) {
    eventEmit('chat.typing', payload);
}

export function emitAlert(payload) {
    eventEmit('chat.alert', payload);
}

// Integration-plane diagnostic events (receipts timeline + workspace health
// badge). Kept separate from chat.* so subscribers can filter independently.
export function emitIntegrationMessage(payload) {
    eventEmit('integration.message', payload);
}

export function emitAutoreplyFailed(payload) {
    eventEmit('integration.autoreply_failed', payload);
}
