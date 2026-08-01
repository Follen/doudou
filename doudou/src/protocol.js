"use strict";

const fsp = require("node:fs/promises");
const path = require("node:path");
const { AppError, ensureDir, now, safeSegment, sha256 } = require("./core");
const { detectLogin, launchSession, openChat } = require("./adapter");
const { loadFingerprint } = require("./fingerprint");
const { buildImageSend, buildTextSend, getConversationState, readConversationPage, resolveMentions, submitCommand, COMMANDS } = require("./im-client");
const { collectMessageMedia, uploadImage } = require("./media");
const { loadProfile, withProfileLock } = require("./profile");
const { readConversationsIndex } = require("./storage");

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
  for (const field of fields) {
    chunks.push(encodeVarint((BigInt(field.number) << 3n) | BigInt(field.wire)));
    if (field.wire === 0) chunks.push(encodeVarint(field.value));
    else if (field.wire === 2) chunks.push(encodeVarint(field.value.length), field.value);
    else chunks.push(field.value);
  }
  return Buffer.concat(chunks);
}

function field(fields, number, wire = null, required = true) {
  const match = fields.find((item) => item.number === number && (wire === null || item.wire === wire));
  if (!match && required) throw new AppError("PROTOBUF_FIELD_MISSING", `Missing protobuf field ${number}.`);
  return match || null;
}

function parseSendResponse(buffer) {
  const text = buffer.toString("utf8");
  if (text.startsWith("{")) return { gateway: JSON.parse(text) };
  const outer = decodeFields(buffer);
  const result = {
    command: field(outer, 1, 0).value.toString(),
    sequence_id: field(outer, 2, 0).value.toString(),
    status_code: Number(field(outer, 3, 0).value),
    status_text: field(outer, 4, 2).value.toString("utf8"),
    server_message_id: null,
    client_message_id: null,
    server_result_code: null,
  };
  const payload = field(outer, 6, 2, false);
  if (!payload) return result;
  const wrapper = decodeFields(payload.value);
  const bodyWrapper = field(wrapper, 100, 2, false);
  if (!bodyWrapper) return result;
  const body = decodeFields(bodyWrapper.value);
  result.server_message_id = field(body, 1, 0, false)?.value.toString() || null;
  result.server_result_code = Number(field(body, 3, 0, false)?.value ?? -1n);
  result.client_message_id = field(body, 4, 2, false)?.value.toString("utf8") || null;
  return result;
}

function artifactTimestamp(iso) {
  return iso.replace(/[-:]/g, "").replace(".", "");
}

function redactEndpoint(value) {
  const url = new URL(value);
  return {
    origin: url.origin,
    path: url.pathname,
    query_keys: [...new Set(url.searchParams.keys())].sort(),
  };
}

function hashOrNull(value) {
  return value === null || value === undefined || value === "" ? null : sha256(String(value));
}

function redactSendResponse(parsed) {
  if (parsed.gateway) return { gateway: "redacted" };
  return {
    command: parsed.command,
    sequence_id: parsed.sequence_id,
    status_code: parsed.status_code,
    status_text: parsed.status_text,
    server_message_id_sha256: hashOrNull(parsed.server_message_id),
    client_message_id_sha256: hashOrNull(parsed.client_message_id),
    server_result_code: parsed.server_result_code,
  };
}

async function waitForValidLogin(session, attempts = 12) {
  let login;
  for (let attempt = 0; attempt < attempts; attempt += 1) {
    login = await detectLogin(session);
    if (login.valid) return login;
    await session.page.waitForTimeout(500);
  }
  return login;
}

async function sendProtocolMessageInSession({ session, dataRoot, profileId, conversation, text = null, image = null, at = [], target = null }) {
  const state = {
    ...getConversationState(session, conversation),
    profileId,
    conversationName: conversation.display_name,
  };
  let upload = null;
  let built;
  if (image) {
    if (target || at.length) throw new AppError("IMAGE_SEND_MODE_INVALID", "Image sends do not support --at or reference targets.");
    upload = await uploadImage(session, image);
    built = buildImageSend(state, upload.content);
  } else {
    const mentions = await resolveMentions(session, state, at);
    built = buildTextSend(state, text, { mentions, target });
  }
  let actualEndpoint = null;
  const observe = (request) => {
    if (request.url().includes("/v1/message/send")) actualEndpoint = redactEndpoint(request.url());
  };
  session.page.on("request", observe);
  let submitted;
  try {
    submitted = await submitCommand(session, state, COMMANDS.SEND_MESSAGE, built.body);
  } finally {
    session.page.off("request", observe);
  }
  const response = submitted.response;
  const responseBody = response.body;
  const sentAt = now();
  const directory = path.join(dataRoot, "message", "profiles", safeSegment(profileId), "sends", sentAt.slice(0, 10), `${artifactTimestamp(sentAt)}__${safeSegment(built.clientMessageId)}`);
  await ensureDir(directory);
  await Promise.all([
    fsp.writeFile(path.join(directory, "01-cli-request.bin"), submitted.request),
    fsp.writeFile(path.join(directory, "02-server-response.bin"), responseBody),
    ...(upload ? [fsp.writeFile(path.join(directory, "03-upload-summary.json"), `${JSON.stringify({ source: upload.source, stages: upload.stages }, null, 2)}\n`)] : []),
  ]);
  const parsed = parseSendResponse(responseBody);
  const checks = {
    http_200: response.status === 200,
    protobuf_status_ok: !parsed.gateway && parsed.status_code === 0,
    server_result_ok: !parsed.gateway && parsed.server_result_code === 0,
    client_message_id_matches: !parsed.gateway && parsed.client_message_id === built.clientMessageId,
    conversation_matches: state.conversationId === String(conversation.conversation_id),
    dom_operations_zero: true,
  };
  const passed = Object.values(checks).every(Boolean);
  const receipt = {
    schema: "doudou.protocol-send-receipt.v1",
    sent_at: sentAt,
    mode: built.mode,
    profile_id: profileId,
    conversation: {
      conversation_id_sha256: hashOrNull(conversation.conversation_id),
      conversation_type: conversation.conversation_type,
      kind: conversation.kind,
    },
    target_message_id_sha256: hashOrNull(target?.message_id),
    text: built.text ? { sha256: sha256(built.text), bytes: Buffer.byteLength(built.text, "utf8") } : null,
    image: upload ? upload.source : null,
    mentions: at.length,
    endpoint: actualEndpoint || { origin: "https://imapi.douyin.com", path: "/v1/message/send", query_keys: [] },
    method: "POST",
    headless: true,
    client_message_id_sha256: hashOrNull(built.clientMessageId),
    server_message_id_sha256: hashOrNull(parsed.server_message_id),
    http_status: response.status,
    http_status_text: response.statusText,
    response: redactSendResponse(parsed),
    files: {
      cli_request: "01-cli-request.bin",
      server_response: "02-server-response.bin",
      upload_summary: upload ? "03-upload-summary.json" : null,
      verification: "05-verification.json",
    },
    sha256: {
      cli_request: sha256(submitted.request),
      server_response: sha256(responseBody),
    },
  };
  const verification = {
    schema: "doudou.protocol-send-verification.v1",
    verified_at: now(),
    passed,
    checks,
    conclusion: passed ? "Server accepted the message and echoed the SDK client message ID." : "One or more transport or Protobuf receipt checks failed.",
  };
  await Promise.all([
    fsp.writeFile(path.join(directory, "04-receipt.json"), `${JSON.stringify(receipt, null, 2)}\n`),
    fsp.writeFile(path.join(directory, "05-verification.json"), `${JSON.stringify(verification, null, 2)}\n`),
  ]);
  if (!passed) throw new AppError("PROTOCOL_SEND_VERIFICATION_FAILED", verification.conclusion, { directory, checks, receipt });
  return { directory, receipt, verification };
}

async function sendProtocolMessages({ dataRoot, profileId, tasks, terminateOwner = false, intervalMs = 5000, onProgress = null }) {
  if (!Array.isArray(tasks) || tasks.length === 0) throw new AppError("INVALID_PROTOCOL_BATCH", "Protocol batch requires at least one task.");
  if (tasks.length > 100) throw new AppError("PROTOCOL_BATCH_TOO_LARGE", "Protocol batch supports at most 100 tasks.", { count: tasks.length, max: 100 });
  if (!Number.isInteger(intervalMs) || intervalMs < 5000) throw new AppError("INVALID_SEND_INTERVAL", "Protocol batch interval must be at least 5000 milliseconds.", { interval_ms: intervalMs });
  return withProfileLock(dataRoot, profileId, { terminateOwner }, async () => {
    const index = await readConversationsIndex(dataRoot, profileId);
    const prepared = tasks.map((task, taskIndex) => {
      const conversation = index.conversations.find((item) => item.conversation_id === String(task.conversationId));
      if (!conversation) throw new AppError("CONVERSATION_NOT_FOUND", `Conversation not found: ${task.conversationId}.`, { task_index: taskIndex + 1 });
      return { ...task, conversation };
    });
    const { paths } = await loadProfile(dataRoot, profileId);
    const fingerprint = await loadFingerprint(paths.fingerprint, { profileId });
    const session = await launchSession({
      browserData: paths.browserData,
      headless: true,
      browser: fingerprint.browser.browser_channel,
      fingerprint,
      fingerprintPath: paths.fingerprint,
    });
    const results = [];
    let failure = null;
    try {
      await openChat(session);
      const login = await waitForValidLogin(session);
      if (!login.valid) throw new AppError(login.reason_code || "LOGIN_REQUIRED", "Profile login is not valid.", { profile_id: profileId });
      for (let index = 0; index < prepared.length; index += 1) {
        const item = prepared[index];
        await onProgress?.({ current: index + 1, total: prepared.length, status: "sending", conversation: item.conversation });
        try {
          const result = await sendProtocolMessageInSession({
            session,
            dataRoot,
            profileId,
            conversation: item.conversation,
            text: item.text,
            image: item.image || null,
            at: item.at || [],
            target: item.target || null,
          });
          results.push({ task_index: index + 1, status: "sent", conversation: item.conversation, ...result });
          await onProgress?.({ current: index + 1, total: prepared.length, status: "sent", conversation: item.conversation, result });
        } catch (error) {
          failure = error;
          results.push({ task_index: index + 1, status: "failed", conversation: item.conversation, error_code: error.code || "SEND_FAILED", message: error.message });
          await onProgress?.({ current: index + 1, total: prepared.length, status: "failed", conversation: item.conversation, error });
          for (let later = index + 1; later < prepared.length; later += 1) {
            results.push({ task_index: later + 1, status: "skipped", conversation: prepared[later].conversation, error_code: "BATCH_STOPPED" });
          }
          break;
        }
        if (index < prepared.length - 1) {
          await onProgress?.({ current: index + 1, total: prepared.length, status: "waiting", wait_ms: intervalMs });
          await session.page.waitForTimeout(intervalMs);
        }
      }
    } finally {
      await session.context.close().catch(() => {});
    }
    return { completed: !failure, interval_ms: intervalMs, results, failure };
  });
}

async function sendProtocolMessage({ dataRoot, profileId, conversationId, text = null, image = null, at = [], target = null, terminateOwner = false }) {
  const batch = await sendProtocolMessages({
    dataRoot,
    profileId,
    tasks: [{ conversationId, text, image, at, target }],
    terminateOwner,
    intervalMs: 5000,
  });
  if (batch.failure) throw batch.failure;
  const [{ conversation, task_index: taskIndex, status, ...result }] = batch.results;
  return result;
}

async function collectProtocolConversation(session, { dataRoot, profileId, group, from, to, runId, maxPages = 80 }) {
  const state = {
    ...getConversationState(session, group),
    profileId,
    conversationName: group.display_name,
    runId,
  };
  const byId = new Map();
  let anchorIndex = 0n;
  let hasMore = true;
  for (let pageNumber = 0; hasMore && pageNumber < maxPages; pageNumber += 1) {
    const page = await readConversationPage(session, state, { anchorIndex, limit: 50, direction: 1 });
    for (const message of page.messages) byId.set(message.message_id, message);
    const dated = [...byId.values()].map((message) => message.sent_at).filter(Boolean).sort();
    if (dated.length && dated[0] <= from) break;
    const next = BigInt(page.nextCursor || 0);
    hasMore = page.hasMore && next !== anchorIndex;
    anchorIndex = next;
  }
  const all = [...byId.values()];
  const unresolved = all.filter((message) => !message.sent_at);
  const messages = all.filter((message) => message.sent_at && message.sent_at >= from && message.sent_at <= to);
  await collectMessageMedia(dataRoot, profileId, group, messages);
  messages.sort((left, right) => left.sent_at.localeCompare(right.sent_at) || left.message_id.localeCompare(right.message_id));
  return { messages, unresolved };
}

module.exports = {
  decodeFields,
  encodeFields,
  parseSendResponse,
  redactEndpoint,
  redactSendResponse,
  collectProtocolConversation,
  sendProtocolMessage,
  sendProtocolMessages,
};
