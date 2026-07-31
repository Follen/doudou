"use strict";

const crypto = require("node:crypto");
const fsp = require("node:fs/promises");
const path = require("node:path");
const { AppError, ensureDir, now, safeSegment, sha256 } = require("./core");
const { SELECTORS, detectLogin, firstVisible, launchSession, openChat, openConversation } = require("./adapter");
const { loadFingerprint } = require("./fingerprint");
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

function stringField(number, value) {
  return { number, wire: 2, value: Buffer.from(String(value), "utf8") };
}

function varintField(number, value) {
  return { number, wire: 0, value: BigInt(value) };
}

function mapEntry(key, value) {
  return encodeFields([stringField(1, key), stringField(2, value)]);
}

function updateMapEntry(raw, key, value) {
  const fields = decodeFields(raw);
  if (field(fields, 1, 2).value.toString("utf8") !== key) return raw;
  field(fields, 2, 2).value = Buffer.from(String(value), "utf8");
  return encodeFields(fields);
}

function replaceSingleField(fields, number, replacement) {
  const index = fields.findIndex((item) => item.number === number);
  if (index === -1) fields.push(replacement);
  else fields.splice(index, 1, replacement);
}

function referenceHint(target) {
  const protocol = target.protocol || {};
  const content = typeof protocol.content === "string" ? protocol.content : JSON.stringify(protocol.content || { text: target.text || "" });
  return {
    refmsg_type: Number(protocol.message_type || 7),
    content: target.text || "",
    refmsg_uid: String(protocol.sender || protocol.sender_uid || target.sender?.id || "0"),
    refmsg_sec_uid: protocol.sec_sender || protocol.sender_sec_uid || target.sender?.sec_uid || "",
    nickname: target.sender?.name || "",
    refmsg_content: content,
    version: Number(protocol.version || 1),
    itemId: "",
    scene_type: 1,
  };
}

function buildReferenceInfo(target) {
  const protocol = target.protocol || {};
  const reference = protocol.reference_info || {};
  const referencedId = String(target.message_id);
  const rootId = String(reference.root_message_id || referencedId);
  const rootIndex = reference.root_message_conv_index || protocol.index_in_conversation;
  if (!/^\d+$/.test(referencedId) || !/^\d+$/.test(String(rootId)) || !/^\d+$/.test(String(rootIndex || ""))) {
    throw new AppError("REFERENCE_PROTOCOL_FIELDS_MISSING", "The collected target lacks numeric message or conversation-index fields required for protocol reply.", {
      message_id: target.message_id,
      required: ["message_id", "protocol.index_in_conversation"],
    });
  }
  const hint = JSON.stringify(referenceHint(target));
  const refContent = typeof protocol.content === "string" ? protocol.content : JSON.stringify(protocol.content || { text: target.text || "" });
  return encodeFields([
    varintField(1, referencedId),
    stringField(2, hint),
    varintField(3, rootId),
    varintField(4, rootIndex),
    { number: 5, wire: 2, value: mapEntry("s:ref_content", refContent) },
    { number: 5, wire: 2, value: mapEntry("s:ref_is_edited", "false") },
  ]);
}

function rewriteSignedSendRequest(freshRequest, text, { target = null } = {}) {
  const outer = decodeFields(freshRequest);
  const wrapperField = field(outer, 8, 2);
  const wrapper = decodeFields(wrapperField.value);
  const bodyField = field(wrapper, 100, 2);
  const body = decodeFields(bodyField.value);
  const contentField = field(body, 4, 2);
  let content;
  try {
    content = JSON.parse(contentField.value.toString("utf8"));
  } catch {
    content = {};
  }
  content.text = text;
  if (target) content.aweType = 703;
  contentField.value = Buffer.from(JSON.stringify(content), "utf8");

  const clientMessageId = field(body, 8, 2).value.toString("utf8") || crypto.randomUUID();
  for (const item of body.filter((entry) => entry.number === 5 && entry.wire === 2)) {
    item.value = updateMapEntry(item.value, "s:client_message_id", clientMessageId);
    item.value = updateMapEntry(item.value, "s:stime", String(Date.now()));
  }
  replaceSingleField(body, 8, stringField(8, clientMessageId));
  if (target) replaceSingleField(body, 11, { number: 11, wire: 2, value: buildReferenceInfo(target) });
  else for (let index = body.length - 1; index >= 0; index -= 1) if (body[index].number === 11) body.splice(index, 1);

  bodyField.value = encodeFields(body);
  wrapperField.value = encodeFields(wrapper);
  return {
    body: encodeFields(outer),
    clientMessageId,
    conversationId: field(body, 1, 2).value.toString("utf8"),
    conversationType: Number(field(body, 2, 0).value),
    messageType: Number(field(body, 6, 0).value),
  };
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

async function pageContextSend(page, url, body) {
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
  }, { requestUrl: url, bodyBase64: body.toString("base64") });
  return { status: result.status, statusText: result.statusText, body: Buffer.from(result.bodyBase64, "base64") };
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

async function sendProtocolMessage({ dataRoot, profileId, conversationId, text, target = null, terminateOwner = false }) {
  return withProfileLock(dataRoot, profileId, { terminateOwner }, async () => {
    const index = await readConversationsIndex(dataRoot, profileId);
    const conversation = index.conversations.find((item) => item.conversation_id === String(conversationId));
    if (!conversation) throw new AppError("CONVERSATION_NOT_FOUND", `Conversation not found: ${conversationId}.`);
    const { paths } = await loadProfile(dataRoot, profileId);
    const fingerprint = await loadFingerprint(paths.fingerprint, { profileId });
    const session = await launchSession({
      browserData: paths.browserData,
      headless: true,
      browser: fingerprint.browser.browser_channel,
      fingerprint,
      fingerprintPath: paths.fingerprint,
    });
    let intercepted;
    let responseBody;
    let responseMeta;
    let routeFailure;
    try {
      await openChat(session);
      const login = await waitForValidLogin(session);
      if (!login.valid) throw new AppError(login.reason_code || "LOGIN_REQUIRED", "Profile login is not valid.", { profile_id: profileId });
      await openConversation(session, conversation);
      await session.page.route("**/v1/message/send*", async (route) => {
        try {
          const request = route.request();
          const originalBody = request.postDataBuffer();
          if (!originalBody) throw new AppError("SIGNED_REQUEST_BODY_MISSING", "The SDK-generated send request has no body.");
          const built = rewriteSignedSendRequest(originalBody, text, { target });
          if (built.conversationId !== String(conversationId) || built.conversationType !== Number(conversation.conversation_type)) {
            throw new AppError("SIGNED_REQUEST_CONVERSATION_MISMATCH", "The SDK-generated request does not match the selected conversation.");
          }
          intercepted = { url: request.url(), method: request.method(), originalBody, ...built };
          await route.abort();
        } catch (error) {
          routeFailure = error;
          await route.abort().catch(() => {});
        }
      }, { times: 1 });

      const composer = await firstVisible(session.page, SELECTORS.composer);
      const send = await firstVisible(session.page, SELECTORS.send);
      if (!composer) throw new AppError("COMPOSER_CONTRACT_DRIFT", "The message composer was not found.");
      await composer.fill(`protocol-bootstrap-${Date.now()}`);
      if (send) await send.click();
      else await composer.press("Enter");
      for (let attempt = 0; attempt < 30 && !intercepted && !routeFailure; attempt += 1) await session.page.waitForTimeout(100);
      if (routeFailure) throw routeFailure;
      if (!intercepted) throw new AppError("SIGNED_SEND_NOT_CAPTURED", "The SDK bootstrap request was not captured before the timeout.");
      const response = await pageContextSend(session.page, intercepted.url, intercepted.body);
      responseBody = response.body;
      responseMeta = { status: response.status, statusText: response.statusText };
    } finally {
      await session.context.close().catch(() => {});
    }

    if (!intercepted || !responseBody) throw new AppError("SIGNED_SEND_NOT_CAPTURED", "The signed send request or response was not captured.");
    const sentAt = now();
    const directory = path.join(dataRoot, "message", "profiles", safeSegment(profileId), "sends", sentAt.slice(0, 10), `${artifactTimestamp(sentAt)}__${safeSegment(intercepted.clientMessageId)}`);
    await ensureDir(directory);
    await Promise.all([
      fsp.writeFile(path.join(directory, "01-sdk-signed-request.bin"), intercepted.originalBody),
      fsp.writeFile(path.join(directory, "02-cli-request.bin"), intercepted.body),
      fsp.writeFile(path.join(directory, "03-server-response.bin"), responseBody),
    ]);
    const parsed = parseSendResponse(responseBody);
    const checks = {
      http_200: responseMeta.status === 200,
      protobuf_status_ok: !parsed.gateway && parsed.status_code === 0,
      server_result_ok: !parsed.gateway && parsed.server_result_code === 0,
      client_message_id_matches: !parsed.gateway && parsed.client_message_id === intercepted.clientMessageId,
      conversation_matches: intercepted.conversationId === String(conversationId),
    };
    const passed = Object.values(checks).every(Boolean);
    const receipt = {
      schema: "douyin-message.protocol-send-receipt.v1",
      sent_at: sentAt,
      mode: target ? "reference_reply" : "plain",
      profile_id: profileId,
      conversation: {
        conversation_id_sha256: hashOrNull(conversation.conversation_id),
        conversation_type: conversation.conversation_type,
        kind: conversation.kind,
      },
      target_message_id_sha256: hashOrNull(target?.message_id),
      text: { sha256: sha256(text), bytes: Buffer.byteLength(text, "utf8") },
      endpoint: redactEndpoint(intercepted.url),
      method: intercepted.method,
      headless: true,
      client_message_id_sha256: hashOrNull(intercepted.clientMessageId),
      server_message_id_sha256: hashOrNull(parsed.server_message_id),
      http_status: responseMeta.status,
      http_status_text: responseMeta.statusText,
      response: redactSendResponse(parsed),
      files: {
        sdk_signed_request: "01-sdk-signed-request.bin",
        cli_request: "02-cli-request.bin",
        server_response: "03-server-response.bin",
        verification: "05-verification.json",
      },
      sha256: {
        sdk_signed_request: sha256(intercepted.originalBody),
        cli_request: sha256(intercepted.body),
        server_response: sha256(responseBody),
      },
    };
    const verification = {
      schema: "douyin-message.protocol-send-verification.v1",
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
  });
}

module.exports = {
  buildReferenceInfo,
  decodeFields,
  encodeFields,
  parseSendResponse,
  redactEndpoint,
  redactSendResponse,
  rewriteSignedSendRequest,
  sendProtocolMessage,
};
