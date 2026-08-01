"use strict";

const crypto = require("node:crypto");
const { AppError } = require("./core");
const { extractProtobufMessages } = require("./adapter");

const COMMANDS = Object.freeze({
  SEND_MESSAGE: 100,
  GET_MESSAGES_BY_CONVERSATION: 301,
  CONVERSATION_PARTICIPANTS_LIST: 605,
});

const ENDPOINTS = Object.freeze({
  [COMMANDS.SEND_MESSAGE]: "/v1/message/send",
  [COMMANDS.GET_MESSAGES_BY_CONVERSATION]: "/v1/message/get_by_conversation",
  [COMMANDS.CONVERSATION_PARTICIPANTS_LIST]: "/v1/conversation/participants_list",
});

function readVarint(buffer, start) {
  let value = 0n;
  let shift = 0n;
  let offset = start;
  while (offset < buffer.length && shift <= 70n) {
    const byte = buffer[offset++];
    value |= BigInt(byte & 0x7f) << shift;
    if (!(byte & 0x80)) return { value, offset };
    shift += 7n;
  }
  throw new AppError("PROTOBUF_INVALID_VARINT", "Invalid protobuf varint.");
}

function decodeFields(buffer) {
  const fields = [];
  let offset = 0;
  while (offset < buffer.length) {
    const key = readVarint(buffer, offset);
    offset = key.offset;
    const number = Number(key.value >> 3n);
    const wire = Number(key.value & 7n);
    if (wire === 0) {
      const scalar = readVarint(buffer, offset);
      fields.push({ number, wire, value: scalar.value });
      offset = scalar.offset;
    } else if (wire === 1 || wire === 5) {
      const length = wire === 1 ? 8 : 4;
      if (offset + length > buffer.length) throw new AppError("PROTOBUF_TRUNCATED", "Truncated fixed-width protobuf field.");
      fields.push({ number, wire, value: buffer.subarray(offset, offset + length) });
      offset += length;
    } else if (wire === 2) {
      const length = readVarint(buffer, offset);
      offset = length.offset;
      const end = offset + Number(length.value);
      if (end > buffer.length) throw new AppError("PROTOBUF_TRUNCATED", "Truncated length-delimited protobuf field.");
      fields.push({ number, wire, value: buffer.subarray(offset, end) });
      offset = end;
    } else {
      throw new AppError("PROTOBUF_WIRE_UNSUPPORTED", `Unsupported protobuf wire type ${wire}.`);
    }
  }
  return fields;
}

function encodeVarint(input) {
  let value = BigInt(input);
  const bytes = [];
  while (value >= 0x80n) {
    bytes.push(Number(value & 0x7fn) | 0x80);
    value >>= 7n;
  }
  bytes.push(Number(value));
  return Buffer.from(bytes);
}

function encodeFields(fields) {
  const chunks = [];
  for (const item of fields) {
    chunks.push(encodeVarint((BigInt(item.number) << 3n) | BigInt(item.wire)));
    if (item.wire === 0) chunks.push(encodeVarint(item.value));
    else if (item.wire === 2) chunks.push(encodeVarint(item.value.length), item.value);
    else chunks.push(item.value);
  }
  return Buffer.concat(chunks);
}

function field(fields, number, wire = null, required = true) {
  const match = fields.find((item) => item.number === number && (wire === null || item.wire === wire));
  if (!match && required) throw new AppError("PROTOBUF_FIELD_MISSING", `Missing protobuf field ${number}.`);
  return match || null;
}

function stringField(number, value) {
  return { number, wire: 2, value: Buffer.from(String(value), "utf8") };
}

function varintField(number, value) {
  return { number, wire: 0, value: BigInt(value) };
}

function mapEntry(key, value) {
  return encodeFields([stringField(1, key), stringField(2, value)]);
}

function replaceField(fields, number, replacement) {
  for (let index = fields.length - 1; index >= 0; index -= 1) {
    if (fields[index].number === number) fields.splice(index, 1);
  }
  fields.push(replacement);
}

function textValue(item) {
  if (!item || item.wire !== 2) return null;
  const value = item.value.toString("utf8");
  return value && !value.includes("\ufffd") && !value.includes("\0") ? value : null;
}

function findInitEnvelope(session) {
  const request = session.capture.requests.findLast((item) => item.url.includes("/v1/message/get_message_by_init"));
  if (!request?.value) throw new AppError("PROTOCOL_INIT_ENVELOPE_MISSING", "The /chat initialization request envelope was not captured.");
  return request.value;
}

function buildCommandRequest(initEnvelope, command, body) {
  const outer = decodeFields(initEnvelope).map((item) => ({ ...item, value: Buffer.isBuffer(item.value) ? Buffer.from(item.value) : item.value }));
  replaceField(outer, 1, varintField(1, command));
  replaceField(outer, 2, varintField(2, BigInt(Date.now()) * 1000n + BigInt(crypto.randomInt(1000))));
  replaceField(outer, 8, { number: 8, wire: 2, value: encodeFields([{ number: command, wire: 2, value: body }]) });
  return encodeFields(outer);
}

function responseBody(buffer, command) {
  const outer = decodeFields(buffer);
  const statusCode = Number(field(outer, 3, 0, false)?.value ?? -1n);
  const statusText = textValue(field(outer, 4, 2, false)) || "";
  const payload = field(outer, 6, 2, false);
  if (!payload) return { statusCode, statusText, body: null, outer };
  const wrapper = decodeFields(payload.value);
  const body = field(wrapper, command, 2, false);
  return { statusCode, statusText, body: body?.value || null, outer };
}

async function pageContextRequest(page, endpoint, body) {
  const result = await page.evaluate(async ({ requestUrl, bodyBase64 }) => {
    const bytes = Uint8Array.from(atob(bodyBase64), (character) => character.charCodeAt(0));
    const response = await fetch(requestUrl, {
      method: "POST",
      credentials: "include",
      headers: { "content-type": "application/x-protobuf" },
      body: bytes,
    });
    const responseBytes = new Uint8Array(await response.arrayBuffer());
    let binary = "";
    for (const byte of responseBytes) binary += String.fromCharCode(byte);
    return { status: response.status, statusText: response.statusText, bodyBase64: btoa(binary) };
  }, { requestUrl: `https://imapi.douyin.com${endpoint}`, bodyBase64: body.toString("base64") });
  return { status: result.status, statusText: result.statusText, body: Buffer.from(result.bodyBase64, "base64") };
}

function parseParticipantsPage(buffer) {
  if (!buffer) return { participants: [], hasMore: false, cursor: "0" };
  const fields = decodeFields(buffer);
  const participants = fields.filter((item) => item.number === 1 && item.wire === 2).map((item) => {
    const record = decodeFields(item.value);
    const ext = {};
    for (const entryField of record.filter((child) => child.number === 8 && child.wire === 2)) {
      const entry = decodeFields(entryField.value);
      const key = textValue(field(entry, 1, 2, false));
      const value = textValue(field(entry, 2, 2, false));
      if (key && value !== null) ext[key] = value;
    }
    return {
      uid: field(record, 1, 0).value.toString(),
      sort_order: field(record, 2, 0, false)?.value.toString() || null,
      role: Number(field(record, 3, 0, false)?.value ?? 0n),
      alias: textValue(field(record, 4, 2, false)) || "",
      sec_uid: textValue(field(record, 5, 2, false)) || "",
      ext,
    };
  });
  return {
    participants,
    hasMore: Boolean(Number(field(fields, 2, 0, false)?.value ?? 0n)),
    cursor: field(fields, 3, 0, false)?.value.toString() || "0",
  };
}

function findConversationInfo(buffer, conversationId) {
  let found = null;
  const visit = (value, depth = 0) => {
    if (found || depth > 10 || !value?.length) return;
    let fields;
    try { fields = decodeFields(value); } catch { return; }
    const id = textValue(field(fields, 1, 2, false));
    const type = field(fields, 3, 0, false)?.value;
    const ticket = textValue(field(fields, 4, 2, false));
    if (id === String(conversationId) && (type === 1n || type === 2n) && ticket) {
      found = {
        conversationId: id,
        shortId: field(fields, 2, 0).value.toString(),
        type: Number(type),
        ticket,
        participants: parseParticipantsPage(field(fields, 6, 2, false)?.value),
      };
      return;
    }
    for (const item of fields) if (item.wire === 2) visit(item.value, depth + 1);
  };
  visit(buffer);
  if (!found) throw new AppError("CONVERSATION_PROTOCOL_STATE_MISSING", "The selected conversation was not present in the /chat initialization protocol state.", { conversation_id: String(conversationId) });
  return found;
}

function getConversationState(session, conversation) {
  const response = session.capture.protobuf.findLast((item) => item.url.includes("/v1/message/get_message_by_init"));
  if (!response?.value) throw new AppError("PROTOCOL_INIT_RESPONSE_MISSING", "The /chat initialization response was not captured.");
  const state = findConversationInfo(response.value, conversation.conversation_id);
  if (state.type !== Number(conversation.conversation_type)) throw new AppError("CONVERSATION_PROTOCOL_TYPE_MISMATCH", "Conversation type differs from the protocol index.");
  return { ...state, initEnvelope: findInitEnvelope(session) };
}

function buildReferenceInfo(target) {
  const protocol = target.protocol || {};
  const reference = protocol.reference_info || {};
  const referencedId = String(target.message_id);
  const rootId = String(reference.root_message_id || referencedId);
  const rootIndex = reference.root_message_conv_index || protocol.index_in_conversation;
  if (!/^\d+$/.test(referencedId) || !/^\d+$/.test(rootId) || !/^\d+$/.test(String(rootIndex || ""))) {
    throw new AppError("REFERENCE_PROTOCOL_FIELDS_MISSING", "The target message lacks protocol reference fields.");
  }
  const rawContent = typeof protocol.content === "string" ? protocol.content : JSON.stringify(protocol.content || { text: target.text || "" });
  const hint = JSON.stringify({
    refmsg_type: Number(protocol.message_type || 7),
    content: target.text || "",
    refmsg_uid: String(protocol.sender || target.sender?.id || "0"),
    refmsg_sec_uid: protocol.sec_sender || target.sender?.sec_uid || "",
    nickname: target.sender?.name || "",
    refmsg_content: rawContent,
    version: Number(protocol.version || 1),
    itemId: "",
    scene_type: 1,
  });
  return encodeFields([
    varintField(1, referencedId),
    stringField(2, hint),
    varintField(3, rootId),
    varintField(4, rootIndex),
    { number: 5, wire: 2, value: mapEntry("s:ref_content", rawContent) },
    { number: 5, wire: 2, value: mapEntry("s:ref_is_edited", "false") },
  ]);
}

function buildTextPayload(text, mentions, conversationId, isReply) {
  const richTextInfos = [];
  let offset = 0;
  for (const mention of mentions) {
    const token = `@${mention.name} `;
    const length = token.length;
    richTextInfos.push(
      { info: { uid: mention.uid, con_id: conversationId }, infoType: 1, length, location: offset },
      { info: { font_weight: 2 }, infoType: 2, length, location: offset },
    );
    offset += length;
  }
  const prefix = mentions.map((mention) => `@${mention.name} `).join("");
  return {
    text: `${prefix}${text}`,
    content: {
      ai_ext: "{}",
      scene: "",
      aweType: isReply ? 703 : mentions.length ? 700 : 0,
      related_share_video: {},
      mention_users: [],
      text: `${prefix}${text}`,
      richTextInfos,
      type: 0,
    },
  };
}

function buildSendBody(state, payload) {
  const clientMessageId = payload.clientMessageId || crypto.randomUUID();
  const timestamp = String(Date.now());
  const fields = [
    stringField(1, state.conversationId),
    varintField(2, state.type),
    varintField(3, state.shortId),
    stringField(4, JSON.stringify(payload.content)),
    { number: 5, wire: 2, value: mapEntry("s:client_message_id", clientMessageId) },
    { number: 5, wire: 2, value: mapEntry("s:stime", timestamp) },
    varintField(6, payload.messageType),
    stringField(7, state.ticket),
    stringField(8, clientMessageId),
  ];
  if (payload.mentions?.length) {
    const ids = payload.mentions.map((item) => item.uid);
    fields.push({ number: 5, wire: 2, value: mapEntry("s:mentioned_users", ids.join(",")) });
    for (const uid of ids) fields.push(varintField(9, uid));
  }
  if (payload.target) fields.push({ number: 11, wire: 2, value: buildReferenceInfo(payload.target) });
  return { body: encodeFields(fields), clientMessageId };
}

function buildTextSend(state, text, { mentions = [], target = null } = {}) {
  const content = buildTextPayload(text, mentions, state.conversationId, Boolean(target));
  const built = buildSendBody(state, { content: content.content, messageType: 7, mentions, target });
  return { ...built, text: content.text, messageType: 7, mode: target ? "reference_reply" : mentions.length ? "mention" : "plain" };
}

function buildImageSend(state, imageContent) {
  const content = { ...imageContent, aweType: 2702 };
  const built = buildSendBody(state, { content, messageType: 27, mentions: [], target: null });
  return { ...built, messageType: 27, mode: "image" };
}

async function submitCommand(session, state, command, commandBody) {
  const request = buildCommandRequest(state.initEnvelope, command, commandBody);
  const response = await pageContextRequest(session.page, ENDPOINTS[command], request);
  const parsed = responseBody(response.body, command);
  if (response.status !== 200 || parsed.statusCode !== 0) {
    throw new AppError("PROTOCOL_REQUEST_FAILED", `Protocol command ${command} failed.`, { command, http_status: response.status, status_code: parsed.statusCode, status_text: parsed.statusText });
  }
  return { request, response, parsed };
}

async function readConversationPage(session, state, { anchorIndex = 0, limit = 50, direction = 1 } = {}) {
  const body = encodeFields([
    stringField(1, state.conversationId),
    varintField(2, state.type),
    varintField(3, state.shortId),
    varintField(4, direction),
    varintField(5, anchorIndex),
    varintField(6, limit),
  ]);
  const result = await submitCommand(session, state, COMMANDS.GET_MESSAGES_BY_CONVERSATION, body);
  const responseFields = decodeFields(result.parsed.body || Buffer.alloc(0));
  return {
    ...result,
    messages: extractProtobufMessages(result.response.body, {
      profileId: state.profileId,
      conversationId: state.conversationId,
      conversationType: state.type,
      conversationName: state.conversationName,
      runId: state.runId,
    }),
    nextCursor: field(responseFields, 2, 0, false)?.value.toString() || "0",
    hasMore: Boolean(Number(field(responseFields, 3, 0, false)?.value ?? 0n)),
  };
}

async function listParticipants(session, state) {
  const output = [...state.participants.participants];
  let cursor = state.participants.cursor;
  let hasMore = state.participants.hasMore;
  for (let page = 0; hasMore && page < 100; page += 1) {
    const body = encodeFields([
      stringField(1, state.conversationId),
      varintField(2, state.shortId),
      varintField(3, state.type),
      varintField(4, cursor),
      varintField(5, 100),
    ]);
    const result = await submitCommand(session, state, COMMANDS.CONVERSATION_PARTICIPANTS_LIST, body);
    const response = decodeFields(result.parsed.body || Buffer.alloc(0));
    const participantPage = parseParticipantsPage(field(response, 1, 2, false)?.value);
    output.push(...participantPage.participants);
    cursor = participantPage.cursor;
    hasMore = participantPage.hasMore;
  }
  return [...new Map(output.map((item) => [item.uid, item])).values()];
}

async function resolveUserNames(page, participants) {
  const secUids = participants.map((item) => item.sec_uid).filter(Boolean);
  if (!secUids.length) return participants;
  const result = await page.evaluate(async ({ values }) => {
    const body = new URLSearchParams({ sec_user_ids: JSON.stringify(values) });
    const response = await fetch("/aweme/v1/web/im/user/info/", {
      method: "POST",
      credentials: "include",
      headers: { "content-type": "application/x-www-form-urlencoded" },
      body,
    });
    return { status: response.status, text: await response.text() };
  }, { values: secUids });
  if (result.status !== 200) throw new AppError("PARTICIPANT_USER_INFO_FAILED", "Participant user-info lookup failed.", { http_status: result.status });
  let root;
  try { root = JSON.parse(result.text); } catch { throw new AppError("PARTICIPANT_USER_INFO_INVALID", "Participant user-info response was not JSON."); }
  const bySecUid = new Map();
  const visit = (value) => {
    if (Array.isArray(value)) return value.forEach(visit);
    if (!value || typeof value !== "object") return;
    const secUid = value.sec_uid || value.secUid;
    const name = value.remark_name || value.remarkName || value.nickname || value.nick_name || value.name;
    if (secUid && name) bySecUid.set(String(secUid), String(name));
    Object.values(value).forEach(visit);
  };
  visit(root);
  return participants.map((item) => ({ ...item, name: item.alias || bySecUid.get(item.sec_uid) || "" }));
}

async function resolveMentions(session, state, requested) {
  if (!requested?.length) return [];
  if (state.type !== 2) throw new AppError("MENTION_REQUIRES_GROUP", "--at is only valid for group conversations.");
  const members = await resolveUserNames(session.page, await listParticipants(session, state));
  const resolved = [];
  for (const query of requested) {
    const matches = members.filter((member) => member.name === query || member.alias === query || member.uid === query || member.sec_uid === query);
    if (matches.length === 0) throw new AppError("MENTION_MEMBER_NOT_FOUND", `Group member not found: ${query}.`, { query });
    if (matches.length > 1) throw new AppError("MENTION_MEMBER_AMBIGUOUS", `Group member is ambiguous: ${query}.`, { query, candidates: matches.map((item) => ({ uid: item.uid, name: item.name })) });
    resolved.push(matches[0]);
  }
  return [...new Map(resolved.map((item) => [item.uid, item])).values()];
}

module.exports = {
  COMMANDS,
  ENDPOINTS,
  buildCommandRequest,
  buildImageSend,
  buildSendBody,
  buildTextPayload,
  buildTextSend,
  decodeFields,
  encodeFields,
  field,
  findConversationInfo,
  getConversationState,
  listParticipants,
  mapEntry,
  pageContextRequest,
  parseParticipantsPage,
  readConversationPage,
  resolveMentions,
  responseBody,
  stringField,
  submitCommand,
  varintField,
};
