"use strict";

const { AppError, now, sleep } = require("./core");
const {
  applyFingerprint,
  baselineRequired,
  collectFingerprintBounded,
  createFingerprintRecord,
  saveFingerprint,
  verifyFingerprintPage,
} = require("./fingerprint");

const CHAT_URL = "https://www.douyin.com/chat";

const SELECTORS = {
  messageEntry: [
    '[data-e2e="im-entry"]',
    '[data-e2e="something-button"]:has-text("消息")',
    'text=消息',
    'a[href*="message"]',
  ],
  conversationItems: [
    "[data-conversation-id]",
    "[data-conversation-short-id]",
    "[data-chat-id]",
    '[data-e2e*="conversation-item"]',
  ],
  messageItems: [
    "[data-message-id]",
    "[data-msg-id]",
    "[data-server-message-id]",
    '[data-e2e*="message-item"]',
  ],
  composer: [
    '[contenteditable="true"][role="textbox"]',
    '[contenteditable="true"]',
    "textarea",
  ],
  send: [
    'button:has-text("发送")',
    '[data-e2e*="send"]',
  ],
};

const MESSAGE_BODY_FIELDS = Object.freeze({
  1: { name: "conversation_id", wire_type: 2, value_type: "string" },
  2: { name: "conversation_type", wire_type: 0, value_type: "int32" },
  3: { name: "server_message_id", wire_type: 0, value_type: "int64" },
  4: { name: "index_in_conversation", wire_type: 0, value_type: "int64" },
  5: { name: "conversation_short_id", wire_type: 0, value_type: "int64" },
  6: { name: "message_type", wire_type: 0, value_type: "int32" },
  7: { name: "sender", wire_type: 0, value_type: "int64" },
  8: { name: "content", wire_type: 2, value_type: "string" },
  9: { name: "ext", wire_type: 2, value_type: "map<string,string>" },
  10: { name: "create_time", wire_type: 0, value_type: "int64" },
  11: { name: "version", wire_type: 0, value_type: "int64" },
  12: { name: "status", wire_type: 0, value_type: "int32" },
  13: { name: "order_in_conversation", wire_type: 0, value_type: "int64" },
  14: { name: "sec_sender", wire_type: 2, value_type: "string" },
  15: { name: "property_list", wire_type: 2, value_type: "map<string,PropertyItemList>" },
  17: { name: "index_in_conversation_v2", wire_type: 0, value_type: "int64" },
  18: { name: "reference_info", wire_type: 2, value_type: "ReferenceInfo" },
});

const REFERENCE_INFO_FIELDS = Object.freeze({
  1: "referenced_message_id",
  2: "hint",
  3: "ref_message_type",
  4: "referenced_message_status",
  5: "root_message_id",
  6: "root_message_conv_index",
  7: "ext",
});

function cleanResourceUrl(value) {
  try {
    const url = new URL(value);
    const pathname = url.pathname
      .split("/")
      .map((part) => (/^(?:\d{6,}|[a-f0-9]{24,}|[A-Za-z0-9_-]{40,})$/i.test(part) ? ":id" : part))
      .join("/");
    return `${url.origin}${pathname}`;
  } catch {
    return String(value).split(/[?#]/, 1)[0];
  }
}

function readProtoVarint(buffer, start) {
  let value = 0n;
  let shift = 0n;
  let offset = start;
  while (offset < buffer.length && shift <= 63n) {
    const byte = buffer[offset];
    value |= BigInt(byte & 0x7f) << shift;
    offset += 1;
    if (!(byte & 0x80)) return { value, offset };
    shift += 7n;
  }
  return null;
}

function parseProtoFields(buffer) {
  const fields = [];
  let offset = 0;
  while (offset < buffer.length) {
    const key = readProtoVarint(buffer, offset);
    if (!key || key.value === 0n) return null;
    offset = key.offset;
    const number = Number(key.value >> 3n);
    const wire = Number(key.value & 7n);
    if (wire === 0) {
      const scalar = readProtoVarint(buffer, offset);
      if (!scalar) return null;
      fields.push({ number, wire, value: scalar.value });
      offset = scalar.offset;
    } else if (wire === 1) {
      if (offset + 8 > buffer.length) return null;
      fields.push({ number, wire, value: buffer.subarray(offset, offset + 8) });
      offset += 8;
    } else if (wire === 2) {
      const length = readProtoVarint(buffer, offset);
      if (!length || length.value > BigInt(buffer.length)) return null;
      offset = length.offset;
      const end = offset + Number(length.value);
      if (end > buffer.length) return null;
      fields.push({ number, wire, value: buffer.subarray(offset, end) });
      offset = end;
    } else if (wire === 5) {
      if (offset + 4 > buffer.length) return null;
      fields.push({ number, wire, value: buffer.subarray(offset, offset + 4) });
      offset += 4;
    } else {
      return null;
    }
  }
  return fields;
}

function protoText(field) {
  if (!field || field.wire !== 2) return null;
  const text = field.value.toString("utf8");
  return text && !text.includes("\ufffd") && !text.includes("\0") ? text : null;
}

function normalizeProtoConversation(buffer) {
  const fields = parseProtoFields(buffer);
  if (!fields) return null;
  const id = protoText(fields.find((field) => field.number === 1));
  const shortId = fields.find((field) => field.number === 2 && field.wire === 0);
  const rawType = fields.find((field) => field.number === 3 && field.wire === 0)?.value;
  const sortOrder = fields.find((field) => field.number === 4 && field.wire === 0);
  const displayName = protoText(fields.find((field) => field.number === 5));
  const validId = rawType === 1n
    ? /^0:1:\d{6,24}:\d{6,24}$/.test(id || "")
    : /^\d{16,24}$/.test(id || "");
  const validDisplayName = rawType === 1n ? !displayName || displayName.length <= 200 : Boolean(displayName && displayName.length <= 200);
  if (!validId || !shortId || !sortOrder || ![1n, 2n].includes(rawType) || !validDisplayName) return null;
  const conversationType = Number(rawType);
  return {
    conversation_id: id,
    conversation_short_id: shortId.value.toString(),
    conversation_type: conversationType,
    kind: conversationType === 2 ? "group" : "direct",
    display_name: displayName?.trim() || (conversationType === 2 ? "未命名群聊" : "未命名私聊"),
    last_message_at: null,
    discovery_status: "active",
    source: "protobuf",
  };
}

function extractProtobufConversations(buffer, maxDepth = 8) {
  const conversations = [];
  const visit = (message, depth) => {
    if (depth > maxDepth) return;
    const fields = parseProtoFields(message);
    if (!fields) return;
    for (const field of fields) {
      if (field.wire !== 2 || field.value.length === 0) continue;
      if (field.number === 50) {
        const conversation = normalizeProtoConversation(field.value);
        if (conversation) conversations.push(conversation);
      }
      if (field.value.length <= 10 * 1024 * 1024) visit(field.value, depth + 1);
    }
  };
  visit(buffer, 0);
  return [...new Map(conversations.map((conversation) => [conversation.conversation_id, conversation])).values()];
}

function extractProtobufGroups(buffer, maxDepth = 8) {
  return extractProtobufConversations(buffer, maxDepth).filter((conversation) => conversation.kind === "group");
}

function protoDecimal(value) {
  if (value === undefined || value === null) return null;
  return value <= BigInt(Number.MAX_SAFE_INTEGER) ? Number(value) : value.toString();
}

function protoStructuredValue(field, depth = 0) {
  if (!field) return null;
  if (field.wire === 0) return protoDecimal(field.value);
  if (field.wire !== 2) return { wire_type: field.wire, byte_length: field.value.length };
  const text = protoText(field);
  if (text && !/[\u0000-\u0008\u000e-\u001f]/.test(text)) return parseContent(text);
  if (depth >= 4) return { wire_type: field.wire, byte_length: field.value.length };
  const nested = parseProtoFields(field.value);
  if (!nested) return { wire_type: field.wire, byte_length: field.value.length };
  const output = {};
  for (const child of nested) {
    const key = String(child.number);
    const value = protoStructuredValue(child, depth + 1);
    if (!(key in output)) output[key] = value;
    else if (Array.isArray(output[key])) output[key].push(value);
    else output[key] = [output[key], value];
  }
  return output;
}

function parseProtoExt(fields) {
  const ext = {};
  for (const field of fields.filter((item) => item.number === 9 && item.wire === 2)) {
    const entry = parseProtoFields(field.value);
    const key = protoText(entry?.find((item) => item.number === 1));
    const value = protoText(entry?.find((item) => item.number === 2));
    if (!key || value === null) continue;
    if (!(key in ext)) ext[key] = value;
    else if (Array.isArray(ext[key])) ext[key].push(value);
    else ext[key] = [ext[key], value];
  }
  return ext;
}

function parseProtoPropertyItem(buffer) {
  const fields = parseProtoFields(buffer) || [];
  return {
    uid: protoDecimal(fields.find((field) => field.number === 1 && field.wire === 0)?.value),
    sec_uid: protoText(fields.find((field) => field.number === 2)),
    create_time: protoDecimal(fields.find((field) => field.number === 3 && field.wire === 0)?.value),
    idempotent_id: protoText(fields.find((field) => field.number === 4)),
    value: protoText(fields.find((field) => field.number === 5)),
    unknown_fields: fields
      .filter((field) => field.number < 1 || field.number > 5)
      .map((field) => ({ field: field.number, wire_type: field.wire, value: protoStructuredValue(field) })),
  };
}

function parseProtoPropertyList(fields) {
  const output = {};
  for (const field of fields.filter((item) => item.number === 15 && item.wire === 2)) {
    const entry = parseProtoFields(field.value);
    const key = protoText(entry?.find((item) => item.number === 1));
    const listField = entry?.find((item) => item.number === 2 && item.wire === 2);
    if (!key || !listField) continue;
    const list = parseProtoFields(listField.value) || [];
    output[key] = {
      items: list
        .filter((item) => item.number === 1 && item.wire === 2)
        .map((item) => parseProtoPropertyItem(item.value)),
      unknown_fields: list
        .filter((item) => item.number !== 1)
        .map((item) => ({ field: item.number, wire_type: item.wire, value: protoStructuredValue(item) })),
    };
  }
  return output;
}

function parseProtoReferenceInfo(field) {
  if (!field || field.wire !== 2) return null;
  const fields = parseProtoFields(field.value);
  if (!fields) return null;
  const hintRaw = protoText(fields.find((item) => item.number === 2));
  const ext = {};
  for (const item of fields.filter((entry) => entry.number === 7 && entry.wire === 2)) {
    const mapEntry = parseProtoFields(item.value);
    const key = protoText(mapEntry?.find((entry) => entry.number === 1));
    const value = protoText(mapEntry?.find((entry) => entry.number === 2));
    if (key && value !== null) ext[key] = value;
  }
  return {
    referenced_message_id: protoDecimal(fields.find((item) => item.number === 1 && item.wire === 0)?.value),
    hint: parseContent(hintRaw),
    hint_raw: hintRaw,
    ref_message_type: protoDecimal(fields.find((item) => item.number === 3 && item.wire === 0)?.value),
    referenced_message_status: protoDecimal(fields.find((item) => item.number === 4 && item.wire === 0)?.value),
    root_message_id: protoDecimal(fields.find((item) => item.number === 5 && item.wire === 0)?.value),
    root_message_conv_index: protoDecimal(fields.find((item) => item.number === 6 && item.wire === 0)?.value),
    ext,
    unknown_fields: fields
      .filter((item) => !(item.number in REFERENCE_INFO_FIELDS))
      .map((item) => ({ field: item.number, wire_type: item.wire, value: protoStructuredValue(item) })),
  };
}

function protocolReplyId(content, ext, referenceInfo) {
  const direct = firstValue(content && typeof content === "object" ? content : {}, ["reply_to_message_id", "replyToMessageId", "quote_message_id", "quoteMessageId", "referenced_message_id"]);
  if (direct !== null) return String(direct);
  for (const key of ["s:reply_message_id", "s:reply_to_message_id", "reply_to_message_id", "s:reference_message_id", "s:quote_message_id"]) {
    if (ext[key]) return String(Array.isArray(ext[key]) ? ext[key][0] : ext[key]);
  }
  if (referenceInfo && typeof referenceInfo === "object") {
    if (referenceInfo.referenced_message_id !== null && referenceInfo.referenced_message_id !== undefined) return String(referenceInfo.referenced_message_id);
    const values = Object.values(referenceInfo).flat(2);
    const candidate = values.find((value) => /^\d{16,24}$/.test(String(value)));
    if (candidate !== undefined) return String(candidate);
  }
  return null;
}

function normalizeProtoMessage(buffer, context) {
  const fields = parseProtoFields(buffer);
  if (!fields) return null;
  const conversationId = protoText(fields.find((field) => field.number === 1));
  const conversationType = fields.find((field) => field.number === 2 && field.wire === 0)?.value;
  const messageId = fields.find((field) => field.number === 3 && field.wire === 0)?.value;
  const sortOrder = fields.find((field) => field.number === 4 && field.wire === 0)?.value;
  const conversationShortId = fields.find((field) => field.number === 5 && field.wire === 0)?.value;
  const messageType = fields.find((field) => field.number === 6 && field.wire === 0)?.value;
  const senderId = fields.find((field) => field.number === 7 && field.wire === 0)?.value;
  const contentText = protoText(fields.find((field) => field.number === 8));
  const content = parseContent(contentText);
  const ext = parseProtoExt(fields);
  const createTime = fields.find((field) => field.number === 10 && field.wire === 0)?.value;
  const version = fields.find((field) => field.number === 11 && field.wire === 0)?.value;
  const status = fields.find((field) => field.number === 12 && field.wire === 0)?.value;
  const orderInConversation = fields.find((field) => field.number === 13 && field.wire === 0)?.value;
  const senderSecUid = protoText(fields.find((field) => field.number === 14));
  const propertyList = parseProtoPropertyList(fields);
  const referenceField = fields.find((field) => field.number === 18);
  const referenceInfo = parseProtoReferenceInfo(referenceField);
  const indexInConversationV2 = fields.find((field) => field.number === 17 && field.wire === 0)?.value;
  const expectedConversationType = Number(context.conversationType || context.conversation_type || 0);
  const supportedConversationType = conversationType === 1n || conversationType === 2n;
  const matchesConversationType = !expectedConversationType || conversationType === BigInt(expectedConversationType);
  if (
    !conversationId || conversationId !== String(context.conversationId) || !supportedConversationType || !matchesConversationType ||
    messageId === undefined || sortOrder === undefined || conversationShortId === undefined || senderId === undefined ||
    messageType === undefined || createTime === undefined || !contentText
  ) return null;
  const normalized = normalizeMessageCandidate({
    conversation_id: conversationId,
    conversation_short_id: conversationShortId.toString(),
    message_id: messageId.toString(),
    message_type: Number(messageType),
    create_time: Number(createTime),
    uid: senderId.toString(),
    content: contentText,
  }, context);
  normalized.sender.sec_uid = senderSecUid;
  normalized.reply_to_message_id = protocolReplyId(content, ext, referenceInfo);
  normalized.protocol = {
    conversation_id: conversationId,
    conversation_type: Number(conversationType),
    server_message_id: messageId.toString(),
    index_in_conversation: protoDecimal(sortOrder),
    conversation_short_id: protoDecimal(conversationShortId),
    message_type: Number(messageType),
    sender: protoDecimal(senderId),
    sender_uid: protoDecimal(senderId),
    sec_sender: senderSecUid,
    sender_sec_uid: senderSecUid,
    content,
    ext,
    create_time: protoDecimal(createTime),
    create_time_ms: protoDecimal(createTime),
    version: protoDecimal(version),
    status: protoDecimal(status),
    order_in_conversation: protoDecimal(orderInConversation),
    property_list: propertyList,
    reference_info: referenceInfo,
    index_in_conversation_v2: protoDecimal(indexInConversationV2),
    unknown_fields: fields
      .filter((field) => !(field.number in MESSAGE_BODY_FIELDS))
      .map((field) => ({ field: field.number, wire_type: field.wire, value: protoStructuredValue(field) })),
  };
  return normalized;
}

function extractProtobufMessages(buffer, context, maxDepth = 10) {
  const messages = [];
  const visit = (message, depth) => {
    if (depth > maxDepth) return;
    const normalized = normalizeProtoMessage(message, context);
    if (normalized) messages.push(normalized);
    const fields = parseProtoFields(message);
    if (!fields) return;
    for (const field of fields) {
      if (field.wire === 2 && field.value.length > 0 && field.value.length <= 10 * 1024 * 1024) visit(field.value, depth + 1);
    }
  };
  visit(buffer, 0);
  return [...new Map(messages.map((message) => [message.message_id, message])).values()];
}

function walkObjects(value, callback, seen = new WeakSet()) {
  if (!value || typeof value !== "object") return;
  if (seen.has(value)) return;
  seen.add(value);
  if (!Array.isArray(value)) callback(value);
  for (const child of Array.isArray(value) ? value : Object.values(value)) walkObjects(child, callback, seen);
}

function firstValue(object, names) {
  for (const name of names) {
    if (object[name] !== undefined && object[name] !== null && object[name] !== "") return object[name];
  }
  return null;
}

function collectSenderDirectory(responses) {
  const directory = new Map();
  for (const response of responses) walkObjects(response.value, (object) => {
    const name = firstValue(object, ["nickname", "nick_name", "nickName", "display_name", "displayName"]);
    if (!name) return;
    for (const key of ["uid", "user_id", "userId", "sec_uid", "secUid"]) {
      if (object[key] !== undefined && object[key] !== null && object[key] !== "") directory.set(String(object[key]), String(name));
    }
  });
  return directory;
}

function enrichSenderNames(messages, responses) {
  const directory = collectSenderDirectory(responses);
  for (const message of messages) walkObjects(message.protocol?.content, (object) => {
    const name = firstValue(object, ["nickname", "nick_name", "nickName", "display_name", "displayName"]);
    if (!name) return;
    for (const key of ["uid", "user_id", "userId", "sec_uid", "secUid"]) {
      if (object[key] !== undefined && object[key] !== null && object[key] !== "") directory.set(String(object[key]), String(name));
    }
  });
  for (const message of messages) {
    if (message.sender.name) continue;
    message.sender.name = directory.get(String(message.sender.id || "")) || directory.get(String(message.sender.sec_uid || "")) || null;
  }
}

function asTimestamp(value) {
  if (value === null || value === undefined || value === "") return null;
  let milliseconds;
  if (typeof value === "number" || /^\d+$/.test(String(value))) {
    const numeric = Number(value);
    milliseconds = numeric < 1e11 ? numeric * 1000 : numeric;
  } else {
    milliseconds = Date.parse(String(value));
  }
  return Number.isFinite(milliseconds) ? new Date(milliseconds).toISOString() : null;
}

function parseContent(value) {
  if (typeof value !== "string") return value;
  const trimmed = value.trim();
  if ((trimmed.startsWith("{") && trimmed.endsWith("}")) || (trimmed.startsWith("[") && trimmed.endsWith("]"))) {
    try {
      return JSON.parse(trimmed);
    } catch {
      return value;
    }
  }
  return value;
}

function collectStrings(value, keys, output = []) {
  if (!value || typeof value !== "object") return output;
  for (const [key, child] of Object.entries(value)) {
    if (keys.has(key) && typeof child === "string" && child.trim()) output.push(child.trim());
    if (child && typeof child === "object") collectStrings(child, keys, output);
  }
  return output;
}

function normalizeConversationCandidate(object) {
  const id = firstValue(object, ["conversation_id", "conversationId", "conversation_short_id", "conversationShortId", "chat_id", "chatId"]);
  if (id === null) return null;
  const rawType = firstValue(object, ["conversation_type", "conversationType", "type", "chat_type", "chatType"]);
  const type = String(rawType ?? "").toLowerCase();
  const isGroup = rawType === 2 || ["2", "group", "group_chat", "groupchat"].includes(type) || Boolean(object.member_count || object.memberCount || object.participants_count);
  const isDirect = rawType === 1 || ["1", "direct", "single", "private", "one_to_one"].includes(type);
  if (!isGroup && !isDirect) return null;
  const conversationType = isGroup ? 2 : 1;
  const displayName = firstValue(object, ["display_name", "displayName", "conversation_name", "conversationName", "name", "title", "group_name", "groupName"]);
  return {
    conversation_id: String(id),
    conversation_short_id: String(firstValue(object, ["conversation_short_id", "conversationShortId"]) || id),
    conversation_type: conversationType,
    kind: isGroup ? "group" : "direct",
    display_name: String(displayName || (isGroup ? "未命名群聊" : "未命名私聊")).trim(),
    last_message_at: asTimestamp(firstValue(object, ["last_message_at", "lastMessageAt", "updated_at", "update_time", "updateTime"])),
    discovery_status: "active",
    source: "structured",
  };
}

function normalizeGroupCandidate(object) {
  const conversation = normalizeConversationCandidate(object);
  return conversation?.kind === "group" ? conversation : null;
}

function normalizeImageUrls(content) {
  const urls = [];
  const visit = (value, parentKey = "") => {
    if (typeof value === "string") {
      if (/url|uri/i.test(parentKey) && /^https?:\/\//i.test(value)) urls.push(value);
      return;
    }
    if (Array.isArray(value)) {
      for (const child of value) visit(child, parentKey);
      return;
    }
    if (!value || typeof value !== "object") return;
    for (const [key, child] of Object.entries(value)) visit(child, key);
  };
  visit(content);
  return [...new Set(urls.filter((url) => /^https?:\/\//i.test(url)))].map((url) => ({ url, source: "structured", may_expire: /[?&](?:sign|token|x-expires|auth)=/i.test(url) || url.includes("?" ) }));
}

function normalizeMessageCandidate(object, context) {
  const messageId = firstValue(object, ["message_id", "messageId", "msg_id", "msgId", "server_message_id", "serverMessageId"]);
  if (messageId === null) return null;
  const conversationId = firstValue(object, ["conversation_id", "conversationId", "conversation_short_id", "conversationShortId", "chat_id", "chatId"]);
  // DOM-only conversation rows currently have no public id. Their synthetic
  // key is still useful for selection, but must not filter structured message
  // payloads that carry the server conversation id.
  if (conversationId !== null && !String(context.conversationId).startsWith("dom:") && String(conversationId) !== String(context.conversationId)) return null;
  const content = parseContent(firstValue(object, ["content", "message_content", "messageContent", "body", "payload"]));
  const textCandidates = [];
  if (typeof content === "string") textCandidates.push(content);
  if (content && typeof content === "object") collectStrings(content, new Set(["text", "content", "title", "description"]), textCandidates);
  const images = content && typeof content === "object" ? normalizeImageUrls(content) : [];
  const rawType = firstValue(object, ["message_type", "messageType", "msg_type", "msgType", "type"]);
  const typeText = String(rawType ?? "").toLowerCase();
  let type = "unsupported";
  const numericType = Number(rawType);
  if (typeText.includes("system") || numericType === 1001 || numericType === 50001) type = "system";
  else if (images.length && textCandidates.length) type = "mixed";
  else if (images.length || typeText.includes("image") || rawType === 2) type = "image";
  else if (textCandidates.length || typeText.includes("text") || rawType === 1) type = "text";
  const senderObject = firstValue(object, ["sender", "from_user", "fromUser", "user", "author"]);
  const sender = senderObject && typeof senderObject === "object" ? senderObject : object;
  const sentAt = asTimestamp(firstValue(object, ["create_time", "createTime", "created_at", "createdAt", "timestamp", "send_time", "sendTime"]));
  const warnings = [];
  if (!sentAt) warnings.push("sent_at_unresolved");
  return {
    schema: "doudou.message.v1",
    profile_id: context.profileId,
    conversation_id: String(context.conversationId),
    conversation_name: context.conversationName,
    message_id: String(messageId),
    sent_at: sentAt,
    sender: {
      id: firstValue(sender, ["uid", "user_id", "userId", "sec_uid", "secUid", "id"])?.toString() || null,
      name: firstValue(sender, ["nickname", "nick_name", "nickName", "name", "display_name", "displayName"])?.toString() || null,
    },
    type,
    text: textCandidates.length ? textCandidates[0] : null,
    images,
    reply_to_message_id: firstValue(object, ["reply_to_message_id", "replyToMessageId", "quote_message_id", "quoteMessageId"])?.toString() || null,
    collected_at: now(),
    run_id: context.runId,
    source: "douyin-web",
    parse_warnings: warnings,
  };
}

class NetworkCapture {
  constructor({ captureScripts = false, maxJsonResponses = 300, bodyTimeoutMs = 4000 } = {}) {
    this.captureScripts = captureScripts;
    this.maxJsonResponses = maxJsonResponses;
    this.bodyTimeoutMs = bodyTimeoutMs;
    this.json = [];
    this.protobuf = [];
    this.requests = [];
    this.scripts = [];
    this.skipped = [];
    this.pending = new Set();
  }

  attach(page) {
    page.on("request", (request) => {
      const contentType = request.headers()["content-type"] || "";
      const body = request.postDataBuffer();
      if (!body || !contentType.includes("protobuf") || this.requests.length >= 100) return;
      this.requests.push({
        url: cleanResourceUrl(request.url()),
        method: request.method(),
        value: body,
      });
    });
    page.on("response", (response) => {
      const promise = this.#capture(response).catch(() => {}).finally(() => this.pending.delete(promise));
      this.pending.add(promise);
    });
  }

  async #capture(response) {
    const request = response.request();
    const resourceType = request.resourceType();
    const contentType = response.headers()["content-type"] || "";
    if (this.captureScripts && resourceType === "script") {
      const body = await this.#body(response);
      this.scripts.push({ url: cleanResourceUrl(response.url()), body, bytes: body.length });
      return;
    }
    const isProtobuf = contentType.includes("protobuf");
    if ((!isProtobuf && this.json.length >= this.maxJsonResponses) || (!isProtobuf && !contentType.includes("json") && resourceType !== "xhr" && resourceType !== "fetch")) return;
    let body;
    try {
      body = await this.#body(response);
    } catch (error) {
      if (error.code === "RESPONSE_BODY_TIMEOUT") {
        this.skipped.push({ url: cleanResourceUrl(response.url()), resource_type: resourceType, status: response.status(), reason_code: error.code });
        return;
      }
      throw error;
    }
    if (body.length > 10 * 1024 * 1024) return;
    if (isProtobuf) {
      if (this.protobuf.length < 100) this.protobuf.push({
        url: cleanResourceUrl(response.url()),
        status: response.status(),
        value: body,
      });
      return;
    }
    try {
      this.json.push({ url: cleanResourceUrl(response.url()), value: JSON.parse(body.toString("utf8")) });
    } catch {
      // Some IM endpoints return protobuf or wrapped text; page doctor records the missing contract separately.
    }
  }

  async #body(response) {
    let timer;
    try {
      return await Promise.race([
        response.body(),
        new Promise((_, reject) => {
          timer = setTimeout(() => reject(new AppError("RESPONSE_BODY_TIMEOUT", "A browser response body did not finish within the capture limit.", { timeout_ms: this.bodyTimeoutMs })), this.bodyTimeoutMs);
        }),
      ]);
    } finally {
      clearTimeout(timer);
    }
  }

  async settle(timeoutMs = 5000) {
    const pending = Promise.allSettled([...this.pending]);
    const settled = await Promise.race([
      pending.then(() => true),
      new Promise((resolve) => setTimeout(() => resolve(false), timeoutMs)),
    ]);
    return { settled, pending: this.pending.size, skipped: this.skipped.length };
  }
}

async function launchSession({ browserData, headless = true, browser = "chromium", captureScripts = false, fingerprint = null, fingerprintPath = null }) {
  const { chromium } = require("playwright");
  const capture = new NetworkCapture({ captureScripts });
  const selectedBrowser = fingerprint?.browser?.browser_channel || browser;
  const options = buildLaunchOptions({ headless, browser: selectedBrowser, fingerprint });
  const context = await chromium.launchPersistentContext(browserData, options);
  try {
    if (fingerprint) {
      const actualVersion = context.browser()?.version() || null;
      if (fingerprint.browser.product_version && actualVersion && fingerprint.browser.product_version !== actualVersion) {
        throw baselineRequired(fingerprint.profile_id, fingerprintPath, "browser_product_version_changed", {
          baseline_version: fingerprint.browser.product_version,
          current_version: actualVersion,
        });
      }
      await applyFingerprint(context, fingerprint);
    }
    const initialPages = context.pages();
    const page = fingerprint ? await context.newPage() : initialPages[0] || await context.newPage();
    if (fingerprint) {
      await Promise.all(initialPages.map((initial) => initial.close().catch(() => {})));
      await verifyFingerprintPage(page, fingerprint);
      if (fingerprintPath) {
        fingerprint.last_verified_at = now();
        fingerprint = await saveFingerprint(fingerprintPath, fingerprint);
      }
    }
    capture.attach(page);
    return { context, page, capture, headless, environment: browserEnvironment(), fingerprint };
  } catch (error) {
    await context.close().catch(() => {});
    throw error;
  }
}

function browserEnvironment() {
  const resolved = Intl.DateTimeFormat().resolvedOptions();
  return {
    locale: "zh-CN",
    timezoneId: resolved.timeZone || "Asia/Shanghai",
    viewport: { width: 1440, height: 900 },
    deviceScaleFactor: 1,
  };
}

function clientHintHeaders(fingerprint) {
  const data = fingerprint?.surfaces?.navigator?.user_agent_data;
  if (!data) return {};
  return {
    "accept-language": fingerprint.surfaces.navigator.languages.map((language, index) => index ? `${language};q=${Math.max(0.1, 1 - index * 0.1).toFixed(1)}` : language).join(","),
    "sec-ch-ua": data.brands.map((item) => `"${item.brand}";v="${item.version}"`).join(", "),
    "sec-ch-ua-mobile": data.mobile ? "?1" : "?0",
    "sec-ch-ua-platform": `"${data.platform}"`,
  };
}

function buildLaunchOptions({ headless, browser = "chromium", fingerprint = null }) {
  const environment = browserEnvironment();
  const options = { headless, ...environment };
  if (browser === "chrome") options.channel = "chrome";
  if (fingerprint) {
    options.userAgent = fingerprint.surfaces.navigator.user_agent;
    options.extraHTTPHeaders = clientHintHeaders(fingerprint);
    options.screen = {
      width: fingerprint.surfaces.screen.width,
      height: fingerprint.surfaces.screen.height,
    };
    options.deviceScaleFactor = fingerprint.surfaces.screen.device_pixel_ratio;
    options.args = ["--disable-blink-features=AutomationControlled", "--enable-webgl", "--use-angle=default"];
    options.ignoreDefaultArgs = ["--enable-automation"];
  }
  return options;
}

async function captureSessionFingerprint({ session, profileId, fingerprintPath, browser = "chromium" }) {
  const probe = await session.context.newPage();
  try {
    const surfaces = await collectFingerprintBounded(probe);
    const record = createFingerprintRecord({
      profileId,
      browserChannel: browser,
      browserVersion: session.context.browser()?.version() || null,
      surfaces,
    });
    return await saveFingerprint(fingerprintPath, record);
  } finally {
    await probe.close().catch(() => {});
  }
}

async function calibrateFingerprint({ browserData, fingerprintPath, profileId, browser = "chromium" }) {
  const session = await launchSession({ browserData, headless: false, browser });
  try {
    return await captureSessionFingerprint({ session, profileId, fingerprintPath, browser });
  } finally {
    await session.context.close().catch(() => {});
  }
}

async function firstVisible(page, selectors) {
  for (const selector of selectors) {
    const locator = page.locator(selector).first();
    if (await locator.count() && await locator.isVisible().catch(() => false)) return locator;
  }
  return null;
}

async function openChat(session) {
  await session.page.goto(CHAT_URL, { waitUntil: "domcontentloaded", timeout: 60000 });
  // The chat route starts the IM client after initial DOM content is ready.
  await session.page.waitForTimeout(8000);
  await session.capture.settle(8000);
}

async function detectLogin(session) {
  const url = session.page.url();
  const title = await session.page.title().catch(() => "");
  const verification = /验证码|安全验证|verification|captcha/i.test(title) || /verify|captcha/i.test(url);
  const verificationPanel = await firstVisible(session.page, [
    '[id*="captcha" i]',
    '[class*="captcha" i]',
    '[id*="verify" i]',
    '[class*="verify" i]',
    'text=/请输入验证码|安全验证|拖动滑块/',
  ]);
  if (verification || verificationPanel) return { valid: false, status: "verification_required", message_entry: false, reason_code: "VERIFICATION_REQUIRED" };
  const loginUrl = /passport|login/i.test(url);
  const loginPanel = await firstVisible(session.page, ['[id^="login-full-panel"]', '[class*="login-full-panel"]']);
  const messageEntry = await firstVisible(session.page, SELECTORS.messageEntry);
  const loginButton = await firstVisible(session.page, ['button:has-text("登录")', 'a:has-text("登录")', '[data-e2e="login-button"]']);
  if (loginUrl || loginPanel || (!messageEntry && loginButton)) return { valid: false, status: "login_required", message_entry: false, reason_code: "LOGIN_REQUIRED" };
  const initializedImSession = (session.capture?.protobuf || []).some((response) => (
    response.url?.includes("/v1/message/get_message_by_init")
    && response.status >= 200
    && response.status < 300
    && response.value?.length > 0
  ));
  if (new URL(url).pathname === "/chat") {
    return initializedImSession
      ? { valid: true, status: "valid", message_entry: false, reason_code: null }
      : { valid: false, status: "login_required", message_entry: false, reason_code: "LOGIN_REQUIRED" };
  }
  return { valid: Boolean(messageEntry), status: messageEntry ? "valid" : "verification_required", message_entry: Boolean(messageEntry), reason_code: messageEntry ? null : "VERIFICATION_REQUIRED" };
}

function normalizeDomConversationSnapshot(snapshot) {
  const hasTypedGroupAttribute = /data-(?:conversation|chat)-type=(?:group|2)\b/.test(snapshot.attributes);
  const hasTypedDirectAttribute = /data-(?:conversation|chat)-type=(?:direct|single|1)\b/.test(snapshot.attributes);
  const isGroup = snapshot.has_group_avatar || (hasTypedGroupAttribute && !snapshot.has_direct_avatar);
  const isDirect = snapshot.has_direct_avatar || (hasTypedDirectAttribute && !snapshot.has_group_avatar);
  if (!isGroup && !isDirect) return null;
  const kind = isGroup ? "group" : "direct";
  const conversationType = isGroup ? 2 : 1;
  const displayName = snapshot.title || snapshot.text.split(/\r?\n/)[0] || (isGroup ? "未命名群聊" : "未命名私聊");
  const id = snapshot.explicit_id || `dom:${kind}:${encodeURIComponent(displayName)}:${snapshot.index}`;
  return {
    conversation_id: id,
    conversation_short_id: snapshot.explicit_id || null,
    conversation_type: conversationType,
    kind,
    display_name: displayName,
    last_message_at: null,
    discovery_status: snapshot.explicit_id ? "active" : "unresolved_id",
    source: "dom",
  };
}

function normalizeDomGroupSnapshot(snapshot) {
  const conversation = normalizeDomConversationSnapshot(snapshot);
  return conversation?.kind === "group" ? conversation : null;
}

async function discoverDomConversations(page) {
  const snapshots = await page.evaluate((selectors) => {
    const nodes = [...new Set(selectors.flatMap((selector) => [...document.querySelectorAll(selector)]))];
    return nodes.map((node, index) => {
      const titleNode = node.querySelector('.conversationConversationItemtitle');
      return {
        index,
        explicit_id: node.getAttribute("data-conversation-id") || node.getAttribute("data-conversation-short-id") || node.getAttribute("data-chat-id"),
        title: (titleNode?.textContent || "").replace(/\s+/g, " ").trim(),
        text: (node.innerText || node.getAttribute("aria-label") || "").trim(),
        attributes: [...node.attributes].map((attribute) => `${attribute.name}=${attribute.value}`).join(" ").toLowerCase(),
        has_group_avatar: Boolean(node.querySelector('.commonConversationIconnoDrag, [class*="groupAvatar"], [class*="GroupAvatar"]')),
        has_direct_avatar: Boolean(node.querySelector('.commonIMAvataravatarContainer')),
      };
    });
  }, SELECTORS.conversationItems);
  return snapshots.map(normalizeDomConversationSnapshot).filter(Boolean);
}

function mergeDiscoveredConversations(structured, dom) {
  const unique = new Map();
  const structuredNames = new Set();
  for (const conversation of structured) {
    const current = unique.get(conversation.conversation_id);
    if (!current || current.source === "dom") unique.set(conversation.conversation_id, conversation);
    structuredNames.add(`${conversation.kind}\0${conversation.display_name.trim()}`);
  }
  for (const conversation of dom) {
    const nameKey = `${conversation.kind}\0${conversation.display_name.trim()}`;
    const current = unique.get(conversation.conversation_id);
    if (current) {
      const unnamed = current.display_name === "未命名私聊" || current.display_name === "未命名群聊";
      if (unnamed) unique.set(conversation.conversation_id, { ...current, display_name: conversation.display_name });
      continue;
    }
    if (structuredNames.has(nameKey)) continue;
    unique.set(conversation.conversation_id, conversation);
  }
  return [...unique.values()];
}

function enrichDirectConversationNames(conversations, jsonResponses) {
  const directory = new Map();
  for (const response of jsonResponses) walkObjects(response.value, (object) => {
    const uid = firstValue(object, ["uid", "user_id", "userId"]);
    const remark = firstValue(object, ["remark_name", "remarkName"]);
    const nickname = firstValue(object, ["nickname", "nick_name", "nickName"]);
    const name = remark || nickname;
    if (uid === null || !name) return;
    const candidate = { name: String(name).trim(), priority: remark ? 2 : 1 };
    if (!candidate.name || candidate.name.length > 200) return;
    const current = directory.get(String(uid));
    if (!current || candidate.priority > current.priority) directory.set(String(uid), candidate);
  });
  const participantCounts = new Map();
  for (const conversation of conversations) {
    const match = conversation.kind === "direct" && conversation.conversation_id.match(/^0:1:(\d+):(\d+)$/);
    if (!match) continue;
    for (const uid of [match[1], match[2]]) participantCounts.set(uid, (participantCounts.get(uid) || 0) + 1);
  }
  const rankedParticipants = [...participantCounts.entries()].sort((left, right) => right[1] - left[1]);
  const selfUid = rankedParticipants[0]?.[1] > (rankedParticipants[1]?.[1] || 0) ? rankedParticipants[0][0] : null;
  return conversations.map((conversation) => {
    if (conversation.kind !== "direct" || conversation.display_name !== "未命名私聊") return conversation;
    const match = conversation.conversation_id.match(/^0:1:(\d+):(\d+)$/);
    if (!match) return conversation;
    const participantUids = selfUid ? [match[1], match[2]].filter((uid) => uid !== selfUid) : [match[1], match[2]];
    const candidates = participantUids.map((uid) => directory.get(uid)).filter(Boolean);
    if (!candidates.length) return conversation;
    candidates.sort((left, right) => right.priority - left.priority);
    if (candidates.length > 1 && candidates[0].priority === candidates[1].priority) return conversation;
    return { ...conversation, display_name: candidates[0].name };
  });
}

async function discoverConversations(session) {
  await session.capture.settle();
  const structured = [];
  for (const response of session.capture.json) walkObjects(response.value, (object) => {
    const conversation = normalizeConversationCandidate(object);
    if (conversation) structured.push(conversation);
  });
  for (const response of session.capture.protobuf) {
    if (response.url.includes("/v1/message/get_message_by_init")) structured.push(...extractProtobufConversations(response.value));
  }
  structured.splice(0, structured.length, ...enrichDirectConversationNames(structured, session.capture.json));
  const dom = await discoverDomConversations(session.page);
  return mergeDiscoveredConversations(structured, dom);
}

async function inspectWarmupSurface(session) {
  const dom = await session.page.evaluate((selectors) => ({
    panel_search: Boolean([...document.querySelectorAll("input")].find((input) => input.placeholder === "搜索" && input.getBoundingClientRect().width > 0)),
    create_group_control: Boolean(document.querySelector(".GroupMangerCreateGroupcreate")),
    conversation_nodes: new Set(selectors.flatMap((selector) => [...document.querySelectorAll(selector)])).size,
  }), SELECTORS.conversationItems);
  const endpointSet = new Set();
  for (const response of [...session.capture.json, ...session.capture.protobuf]) endpointSet.add(cleanResourceUrl(response.url));
  return {
    dom,
    network: {
      json_response_count: session.capture.json.length,
      protobuf_response_count: session.capture.protobuf.length,
      skipped_response_count: session.capture.skipped.length,
      endpoints: [...endpointSet].sort(),
    },
  };
}

async function discoverGroups(session) {
  return (await discoverConversations(session)).filter((conversation) => conversation.kind === "group");
}

module.exports = {
  CHAT_URL,
  NetworkCapture,
  SELECTORS,
  asTimestamp,
  browserEnvironment,
  buildLaunchOptions,
  calibrateFingerprint,
  captureSessionFingerprint,
  clientHintHeaders,
  cleanResourceUrl,
  detectLogin,
  discoverGroups,
  discoverConversations,
  extractProtobufGroups,
  extractProtobufConversations,
  extractProtobufMessages,
  firstVisible,
  launchSession,
  MESSAGE_BODY_FIELDS,
  enrichDirectConversationNames,
  inspectWarmupSurface,
  mergeDiscoveredConversations,
  normalizeGroupCandidate,
  normalizeDomGroupSnapshot,
  normalizeConversationCandidate,
  normalizeDomConversationSnapshot,
  normalizeMessageCandidate,
  openChat,
  walkObjects,
};
