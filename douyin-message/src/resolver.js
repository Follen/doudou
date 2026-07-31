"use strict";

const { AppError } = require("./core");

function normalizeLookupText(value) {
  return String(value || "").normalize("NFKC").replace(/\s+/gu, " ").trim().toLocaleLowerCase();
}

function conversationCandidate(conversation, index) {
  return {
    number: index + 1,
    conversation_id: conversation.conversation_id,
    conversation_short_id: conversation.conversation_short_id,
    conversation_type: conversation.conversation_type,
    kind: conversation.kind,
    display_name: conversation.display_name,
  };
}

function resolveConversationTarget(index, target) {
  const raw = String(target || "").trim();
  if (!raw) throw new AppError("MISSING_ARGUMENT", "Missing a conversation id or name.", { arguments: ["conversation-id", "conversation", "recipient"] });
  const byId = index.conversations.find((conversation) => conversation.conversation_id === raw);
  if (byId) return { conversation: byId, matched_by: "id", candidates: [conversationCandidate(byId, index.conversations.indexOf(byId))] };
  const normalized = normalizeLookupText(raw);
  const matches = index.conversations
    .map((conversation, position) => ({ conversation, position }))
    .filter(({ conversation }) => normalizeLookupText(conversation.display_name).includes(normalized));
  if (!matches.length) {
    throw new AppError("RECIPIENT_NOT_FOUND", `No existing conversation matches: ${raw}.`, { target: raw, scope: "existing_conversations", candidates: [] });
  }
  const candidates = matches.map(({ conversation, position }) => conversationCandidate(conversation, position));
  if (matches.length > 1) {
    throw new AppError("CONVERSATION_AMBIGUOUS", `More than one existing conversation matches: ${raw}.`, { target: raw, candidates });
  }
  return { conversation: matches[0].conversation, matched_by: "name_contains", candidates };
}

function messageCandidate(message, index) {
  return {
    number: index + 1,
    message_id: message.message_id,
    sender: message.sender,
    sent_at: message.sent_at,
    type: message.type,
    text: message.text || null,
  };
}

function resolveMessageTarget(messages, query) {
  const raw = String(query || "").trim();
  if (!raw) throw new AppError("MISSING_ARGUMENT", "Missing a message id or message keyword.", { arguments: ["message-id", "message-text"] });
  const byId = messages.find((message) => String(message.message_id) === raw);
  if (byId) return { message: byId, matched_by: "id", candidates: [messageCandidate(byId, messages.indexOf(byId))] };
  const normalized = normalizeLookupText(raw);
  const matches = messages
    .map((message, position) => ({ message, position }))
    .filter(({ message }) => normalizeLookupText(message.text).includes(normalized));
  if (!matches.length) throw new AppError("MESSAGE_NOT_FOUND", `No collected message contains: ${raw}.`, { query: raw, candidates: [] });
  const candidates = matches.map(({ message, position }) => messageCandidate(message, position));
  if (matches.length > 1) throw new AppError("MESSAGE_AMBIGUOUS", `More than one message contains: ${raw}.`, { query: raw, candidates });
  return { message: matches[0].message, matched_by: "text_contains", candidates };
}

function defaultMessageWindow(clock = new Date()) {
  const end = new Date(clock);
  const start = new Date(clock);
  start.setHours(0, 0, 0, 0);
  return { from: start.toISOString(), to: end.toISOString(), defaulted: true };
}

module.exports = {
  defaultMessageWindow,
  normalizeLookupText,
  resolveConversationTarget,
  resolveMessageTarget,
};
