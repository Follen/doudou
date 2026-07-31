"use strict";

const assert = require("node:assert/strict");
const test = require("node:test");
const { MESSAGE_BODY_FIELDS, NetworkCapture, buildLaunchOptions, detectLogin, enrichDirectConversationNames, extractProtobufConversations, extractProtobufGroups, extractProtobufMessages, locateMessageByText, mergeDiscoveredConversations, normalizeConversationCandidate, normalizeDomConversationSnapshot, normalizeDomGroupSnapshot, normalizeGroupCandidate, normalizeMessageCandidate } = require("../src/adapter");
const { fieldDiff, inventoryValues, redactShape } = require("../src/page-doctor");
const { parseGroupSelection } = require("../src/commands");
const { decodeFields, encodeFields, redactEndpoint, redactSendResponse, rewriteSignedSendRequest } = require("../src/protocol");

function chatLoginSession(protobuf = [], { verificationPanel = false } = {}) {
  const invisibleLocator = {
    count: async () => 0,
    isVisible: async () => false,
  };
  const visibleLocator = {
    count: async () => 1,
    isVisible: async () => true,
  };
  return {
    page: {
      url: () => "https://www.douyin.com/chat",
      title: async () => "Douyin Chat",
      locator: (selector) => ({ first: () => (verificationPanel && selector.includes("验证码") ? visibleLocator : invisibleLocator) }),
    },
    capture: { protobuf },
  };
}

test("chat route without IM initialization remains login-required", async () => {
  const login = await detectLogin(chatLoginSession());
  assert.deepEqual(login, {
    valid: false,
    status: "login_required",
    message_entry: false,
    reason_code: "LOGIN_REQUIRED",
  });
});

test("chat route requires a successful nonempty IM initialization response", async () => {
  const denied = await detectLogin(chatLoginSession([{
    url: "https://imapi.douyin.com/v1/message/get_message_by_init",
    status: 401,
    value: Buffer.from([0x08, 0x02]),
  }]));
  assert.equal(denied.valid, false);

  const login = await detectLogin(chatLoginSession([{
    url: "https://imapi.douyin.com/v1/message/get_message_by_init",
    status: 200,
    value: Buffer.from([0x08, 0x02]),
  }]));
  assert.equal(login.valid, true);
  assert.equal(login.status, "valid");
});

test("visible captcha blocks login even when an old IM initialization response exists", async () => {
  const login = await detectLogin(chatLoginSession([{
    url: "https://imapi.douyin.com/v1/message/get_message_by_init",
    status: 200,
    value: Buffer.from([0x08, 0x02]),
  }], { verificationPanel: true }));
  assert.deepEqual(login, {
    valid: false,
    status: "verification_required",
    message_entry: false,
    reason_code: "VERIFICATION_REQUIRED",
  });
});

test("structured group and message candidates normalize stable identifiers", () => {
  const group = normalizeGroupCandidate({ conversation_id: "c1", conversation_type: 2, name: "测试群", update_time: 1760000000 });
  assert.equal(group.conversation_id, "c1");
  assert.equal(group.display_name, "测试群");
  const normalized = normalizeMessageCandidate({
    message_id: "m1",
    conversation_id: "c1",
    create_time: 1760000000,
    sender: { uid: "u1", nickname: "User" },
    content: JSON.stringify({ text: "hello", image: { url: "https://example.test/a.jpg?token=secret" } }),
  }, { profileId: "work", conversationId: "c1", conversationName: "测试群", runId: "run-1" });
  assert.equal(normalized.message_id, "m1");
  assert.equal(normalized.type, "mixed");
  assert.equal(normalized.images[0].may_expire, true);
});

test("structured and DOM discovery retain direct conversations", () => {
  const structured = normalizeConversationCandidate({ conversation_id: "d1", conversation_type: 1, name: "私聊用户" });
  assert.equal(structured.kind, "direct");
  assert.equal(structured.conversation_type, 1);
  const dom = normalizeDomConversationSnapshot({
    index: 1,
    explicit_id: "d2",
    title: "私聊用户二",
    text: "私聊用户二",
    attributes: "data-e2e=conversation-item",
    has_group_avatar: false,
    has_direct_avatar: true,
  });
  assert.equal(dom.kind, "direct");
  assert.equal(dom.conversation_type, 1);
});

test("message normalization handles protobuf image lists and system types", () => {
  const context = { profileId: "work", conversationId: "c1", conversationName: "测试群", runId: "run-1" };
  const image = normalizeMessageCandidate({
    message_id: "image-1",
    conversation_id: "c1",
    message_type: 27,
    create_time: 1785494454972,
    content: JSON.stringify({ resource_url: { large_url_list: ["https://example.test/large.jpg?x-signature=secret"] } }),
  }, context);
  assert.equal(image.type, "image");
  assert.equal(image.images.length, 1);
  assert.equal(image.images[0].may_expire, true);

  const system = normalizeMessageCandidate({
    message_id: "system-1",
    conversation_id: "c1",
    message_type: 1001,
    create_time: 1785494454972,
    content: JSON.stringify({ text: "joined" }),
  }, context);
  assert.equal(system.type, "system");
});

test("DOM group discovery uses conversation structure instead of title wording", () => {
  const group = normalizeDomGroupSnapshot({
    index: 0,
    explicit_id: null,
    title: "地堡每日播报",
    text: "地堡每日播报\n16:09\nlatest message",
    attributes: "data-e2e=conversation-item",
    has_group_avatar: true,
    has_direct_avatar: false,
  });
  assert.equal(group.display_name, "地堡每日播报");
  assert.match(group.conversation_id, /^dom:/);

  const directMessageWithGroupInName = normalizeDomGroupSnapshot({
    index: 1,
    explicit_id: null,
    title: "群文件助手",
    text: "群文件助手\n16:02",
    attributes: "data-e2e=conversation-item",
    has_group_avatar: false,
    has_direct_avatar: true,
  });
  assert.equal(directMessageWithGroupInName, null);
});

test("protobuf group discovery uses server conversation type and id", () => {
  const varint = (input) => {
    let value = BigInt(input);
    const bytes = [];
    do {
      let byte = Number(value & 0x7fn);
      value >>= 7n;
      if (value) byte |= 0x80;
      bytes.push(byte);
    } while (value);
    return Buffer.from(bytes);
  };
  const scalar = (field, value) => Buffer.concat([varint(field << 3), varint(value)]);
  const bytes = (field, value) => {
    const body = Buffer.isBuffer(value) ? value : Buffer.from(value);
    return Buffer.concat([varint((field << 3) | 2), varint(body.length), body]);
  };
  const conversation = (id, type, title) => Buffer.concat([
    bytes(1, id),
    scalar(2, 1234567890123456789n),
    scalar(3, type),
    scalar(4, 1785485387590n),
    bytes(5, title),
  ]);
  const payload = bytes(6, Buffer.concat([
    bytes(50, conversation("7606903474747408896", 2, "地堡每日播报")),
    bytes(50, conversation("0:1:111111111111:222222222222", 1, "")),
  ]));
  assert.deepEqual(extractProtobufGroups(payload), [{
    conversation_id: "7606903474747408896",
    conversation_short_id: "1234567890123456789",
    conversation_type: 2,
    kind: "group",
    display_name: "地堡每日播报",
    last_message_at: null,
    discovery_status: "active",
    source: "protobuf",
  }]);
  assert.deepEqual(extractProtobufConversations(payload).map((item) => [item.kind, item.conversation_id]), [
    ["group", "7606903474747408896"],
    ["direct", "0:1:111111111111:222222222222"],
  ]);
});

test("structured groups do not suppress bounded DOM direct conversations", () => {
  const structured = [{
    conversation_id: "7606903474747408896",
    conversation_short_id: "7606903474747408896",
    conversation_type: 2,
    kind: "group",
    display_name: "地堡每日播报",
    source: "protobuf",
  }];
  const dom = [
    { conversation_id: "dom:group:duplicate:0", conversation_type: 2, kind: "group", display_name: "地堡每日播报", source: "dom" },
    { conversation_id: "7606903474747408897", conversation_type: 1, kind: "direct", display_name: "测试联系人", source: "dom" },
  ];
  assert.deepEqual(mergeDiscoveredConversations(structured, dom).map((item) => [item.kind, item.display_name]), [
    ["group", "地堡每日播报"],
    ["direct", "测试联系人"],
  ]);
});

test("DOM titles enrich protocol conversations without replacing their ids", () => {
  const structured = [{
    conversation_id: "0:1:111111111111:222222222222",
    conversation_short_id: "7571160632481923611",
    conversation_type: 1,
    kind: "direct",
    display_name: "未命名私聊",
    source: "protobuf",
  }];
  const dom = [{
    conversation_id: "0:1:111111111111:222222222222",
    conversation_type: 1,
    kind: "direct",
    display_name: "测试联系人",
    source: "dom",
  }];
  assert.deepEqual(mergeDiscoveredConversations(structured, dom), [{
    ...structured[0],
    display_name: "测试联系人",
  }]);
});

test("user info enriches a direct protocol id with its remark name", () => {
  const conversation = (id) => ({
    conversation_id: id,
    conversation_short_id: "7571160632481923611",
    conversation_type: 1,
    kind: "direct",
    display_name: "未命名私聊",
    source: "protobuf",
  });
  const conversations = [
    conversation("0:1:111111111111:222222222222"),
    conversation("0:1:111111111111:333333333333"),
  ];
  const json = [{ value: { data: [
    { uid: "111111111111", nickname: "当前账号" },
    { uid: "222222222222", nickname: "测试昵称", remark_name: "测试联系人" },
  ] } }];
  assert.equal(enrichDirectConversationNames(conversations, json)[0].display_name, "测试联系人");
  assert.equal(enrichDirectConversationNames(conversations, json)[1].display_name, "未命名私聊");
});

test("protocol request rewriting supports plain direct sends and reference replies", () => {
  const textField = (number, value) => ({ number, wire: 2, value: Buffer.from(String(value)) });
  const intField = (number, value) => ({ number, wire: 0, value: BigInt(value) });
  const map = (key, value) => encodeFields([textField(1, key), textField(2, value)]);
  const sendBody = encodeFields([
    textField(1, "direct-conversation"),
    intField(2, 1),
    intField(3, 123),
    textField(4, JSON.stringify({ aweType: 700, text: "bootstrap" })),
    { number: 5, wire: 2, value: map("s:client_message_id", "client-1") },
    { number: 5, wire: 2, value: map("s:stime", "1") },
    intField(6, 7),
    textField(7, "ticket"),
    textField(8, "client-1"),
  ]);
  const request = encodeFields([
    intField(1, 100),
    intField(2, 10),
    { number: 8, wire: 2, value: encodeFields([{ number: 100, wire: 2, value: sendBody }]) },
  ]);
  const plain = rewriteSignedSendRequest(request, "hello");
  assert.equal(plain.conversationType, 1);
  const plainOuter = decodeFields(plain.body);
  const plainWrapper = decodeFields(plainOuter.find((item) => item.number === 8).value);
  const plainBody = decodeFields(plainWrapper.find((item) => item.number === 100).value);
  assert.equal(JSON.parse(plainBody.find((item) => item.number === 4).value).text, "hello");
  assert.equal(plainBody.some((item) => item.number === 11), false);

  const target = {
    message_id: "7668642109649667633",
    text: "target",
    sender: { id: "42", name: "User", sec_uid: "sec-user" },
    protocol: { message_type: 7, index_in_conversation: "1672502523000000", content: { text: "target" } },
  };
  const reply = rewriteSignedSendRequest(request, "reply", { target });
  const replyOuter = decodeFields(reply.body);
  const replyWrapper = decodeFields(replyOuter.find((item) => item.number === 8).value);
  const replyBody = decodeFields(replyWrapper.find((item) => item.number === 100).value);
  const reference = decodeFields(replyBody.find((item) => item.number === 11).value);
  assert.equal(reference.find((item) => item.number === 1).value.toString(), target.message_id);
  assert.equal(JSON.parse(replyBody.find((item) => item.number === 4).value).aweType, 703);
});

test("send receipt helpers retain only endpoint shape and identifier hashes", () => {
  const endpoint = redactEndpoint("https://imapi.douyin.com/v1/message/send?msToken=secret&a_bogus=signed");
  assert.deepEqual(endpoint, {
    origin: "https://imapi.douyin.com",
    path: "/v1/message/send",
    query_keys: ["a_bogus", "msToken"],
  });
  const response = redactSendResponse({
    command: "100",
    sequence_id: "12",
    status_code: 0,
    status_text: "OK",
    server_message_id: "server-secret",
    client_message_id: "client-secret",
    server_result_code: 0,
  });
  const serialized = JSON.stringify(response);
  assert.doesNotMatch(serialized, /server-secret|client-secret/);
  assert.equal(response.status_code, 0);
  assert.equal(response.server_result_code, 0);
});

test("protobuf message discovery extracts server ids, timestamps and content", () => {
  const varint = (input) => {
    let value = BigInt(input);
    const output = [];
    do {
      let byte = Number(value & 0x7fn);
      value >>= 7n;
      if (value) byte |= 0x80;
      output.push(byte);
    } while (value);
    return Buffer.from(output);
  };
  const scalar = (field, value) => Buffer.concat([varint(field << 3), varint(value)]);
  const bytes = (field, value) => {
    const body = Buffer.isBuffer(value) ? value : Buffer.from(value);
    return Buffer.concat([varint((field << 3) | 2), varint(body.length), body]);
  };
  const record = Buffer.concat([
    bytes(1, "7346867556508123660"),
    scalar(2, 2),
    scalar(3, 900000000000000001n),
    scalar(4, 1672502522820000n),
    scalar(5, 7346867556508123660n),
    scalar(6, 7),
    scalar(7, 700000000000000001n),
    bytes(8, JSON.stringify({ text: "只想关掉时间" })),
    bytes(9, Buffer.concat([bytes(1, "s:server_message_create_time"), bytes(2, "1785494454972")])),
    scalar(10, 1785494454972n),
    scalar(11, 3),
    scalar(12, 0),
    scalar(13, 1672502522820000000n),
    bytes(14, "MS4wLjABAAAA-sender"),
    bytes(15, Buffer.concat([
      bytes(1, "reaction"),
      bytes(2, bytes(1, Buffer.concat([
        scalar(1, 700000000000000002n),
        bytes(2, "MS4wLjABAAAA-reactor"),
        scalar(3, 1785494454n),
        bytes(4, "reaction-1"),
        bytes(5, "like"),
      ]))),
    ])),
    scalar(16, 1),
    scalar(17, 12282),
    bytes(18, Buffer.concat([
      scalar(1, 899999999999999999n),
      bytes(2, JSON.stringify({ content: "被引用消息" })),
      scalar(3, 7),
      scalar(4, 0),
      scalar(8, 1672502522810000n),
      scalar(9, 12281),
    ])),
    scalar(20, 9),
  ]);
  const messages = extractProtobufMessages(bytes(2, record), {
    profileId: "work",
    conversationId: "7346867556508123660",
    conversationType: 2,
    conversationName: "露露缇娅的粉丝群1",
    runId: "run-test",
  });
  assert.equal(messages.length, 1);
  assert.equal(messages[0].message_id, "900000000000000001");
  assert.equal(messages[0].sender.id, "700000000000000001");
  assert.equal(messages[0].sender.sec_uid, "MS4wLjABAAAA-sender");
  assert.equal(messages[0].sent_at, "2026-07-31T10:40:54.972Z");
  assert.equal(messages[0].text, "只想关掉时间");
  assert.equal(messages[0].protocol.ext["s:server_message_create_time"], "1785494454972");
  assert.equal(messages[0].protocol.version, 3);
  assert.equal(messages[0].protocol.order_in_conversation, "1672502522820000000");
  assert.equal(messages[0].protocol.server_message_id, "900000000000000001");
  assert.equal(messages[0].protocol.sender, "700000000000000001");
  assert.equal(messages[0].protocol.sec_sender, "MS4wLjABAAAA-sender");
  assert.equal(messages[0].protocol.index_in_conversation_v2, 12282);
  assert.equal(messages[0].protocol.property_list.reaction.items[0].value, "like");
  assert.equal(messages[0].reply_to_message_id, "899999999999999999");
  assert.equal(messages[0].protocol.reference_info.referenced_message_id, "899999999999999999");
  assert.equal(messages[0].protocol.reference_info.ref_message_type, 7);
  assert.deepEqual(messages[0].protocol.reference_info.unknown_fields, [
    { field: 8, wire_type: 0, value: 1672502522810000 },
    { field: 9, wire_type: 0, value: 12281 },
  ]);
  assert.deepEqual(messages[0].protocol.unknown_fields, [
    { field: 16, wire_type: 0, value: 1 },
    { field: 20, wire_type: 0, value: 9 },
  ]);
  assert.equal(MESSAGE_BODY_FIELDS[17].name, "index_in_conversation_v2");
  assert.equal(MESSAGE_BODY_FIELDS[18].name, "reference_info");
  assert.equal(MESSAGE_BODY_FIELDS[16], undefined);

  const directRecord = Buffer.from(record);
  const typeOffset = directRecord.indexOf(scalar(2, 2));
  assert.notEqual(typeOffset, -1);
  directRecord[typeOffset + scalar(2, 2).length - 1] = 1;
  const directMessages = extractProtobufMessages(bytes(2, directRecord), {
    profileId: "work",
    conversationId: "7346867556508123660",
    conversationType: 1,
    conversationName: "私聊用户",
    runId: "run-direct-test",
  });
  assert.equal(directMessages.length, 1);
  assert.equal(directMessages[0].protocol.conversation_type, 1);
  assert.equal(extractProtobufMessages(bytes(2, directRecord), {
    profileId: "work",
    conversationId: "7346867556508123660",
    conversationType: 2,
    conversationName: "群聊",
    runId: "run-mismatch-test",
  }).length, 0);
});

test("headed and headless modes share one stable browser environment", () => {
  const headed = buildLaunchOptions({ headless: false });
  const headless = buildLaunchOptions({ headless: true });
  assert.deepEqual({ ...headed, headless: undefined }, { ...headless, headless: undefined });
  assert.equal(headed.locale, "zh-CN");
  assert.equal(headed.deviceScaleFactor, 1);
  assert.equal("userAgent" in headed, false);
  assert.equal("args" in headed, false);
});

test("network capture skips a response body that never completes", async () => {
  const capture = new NetworkCapture({ bodyTimeoutMs: 20 });
  let responseHandler;
  capture.attach({ on(event, handler) { if (event === "response") responseHandler = handler; } });
  const response = {
    url: () => "https://mcs.zijieapi.com/list",
    status: () => 200,
    request: () => ({ resourceType: () => "fetch" }),
    headers: () => ({ "content-type": "application/json" }),
    body: () => new Promise(() => {}),
  };
  responseHandler(response);
  const result = await capture.settle(100);
  assert.equal(result.settled, true);
  assert.equal(capture.skipped[0].url, "https://mcs.zijieapi.com/list");
  assert.equal(capture.skipped[0].reason_code, "RESPONSE_BODY_TIMEOUT");
});

test("network capture retains protobuf IM responses for typed discovery", async () => {
  const capture = new NetworkCapture();
  let responseHandler;
  capture.attach({ on(event, handler) { if (event === "response") responseHandler = handler; } });
  const body = Buffer.from([0x08, 0x02]);
  responseHandler({
    url: () => "https://imapi.douyin.com/v1/message/get_message_by_init",
    status: () => 200,
    request: () => ({ resourceType: () => "fetch" }),
    headers: () => ({ "content-type": "application/x-protobuf" }),
    body: async () => body,
  });
  await capture.settle(100);
  assert.equal(capture.protobuf.length, 1);
  assert.deepEqual(capture.protobuf[0].value, body);
});

test("group selection supports single, multiple, range and all", () => {
  assert.deepEqual(parseGroupSelection("1", 5), [1]);
  assert.deepEqual(parseGroupSelection("1,3-5,3", 5), [1, 3, 4, 5]);
  assert.deepEqual(parseGroupSelection("全部", 3), [1, 2, 3]);
  assert.throws(() => parseGroupSelection("4-2", 5), (error) => error.code === "INVALID_GROUP_RANGE");
  assert.throws(() => parseGroupSelection("6", 5), (error) => error.code === "GROUP_NUMBER_OUT_OF_RANGE");
});

test("field inventory ignores array indexes and reports field-level drift", () => {
  const baseline = inventoryValues([{ messages: [{ id: "1", text: "a" }] }]);
  const current = inventoryValues([{ messages: [{ id: 1, extra: true, text: null }] }]);
  assert.ok(current.some((field) => field.path === "/messages/*/id"));
  const diff = fieldDiff(baseline, current);
  assert.ok(diff.added.some((field) => field.path === "/messages/*/extra"));
  assert.ok(diff.type_changed.some((field) => field.path === "/messages/*/id"));
  assert.ok(diff.nullability_changed.some((field) => field.path === "/messages/*/text"));
});

test("redacted samples retain shape without raw text, tokens or URL queries", () => {
  const raw = {
    token: "secret-token",
    content: "a private message",
    avatar_url: "https://example.test/user.jpg?token=secret",
    enabled: true,
  };
  const redacted = redactShape(raw);
  const serialized = JSON.stringify(redacted);
  assert.equal(redacted.token.type, "redacted-secret");
  assert.equal(redacted.enabled.value, true);
  assert.doesNotMatch(serialized, /secret-token|private message|\?token=/);
});

test("text fallback finds the bounded message ancestor with a visible reply action", async (t) => {
  const { chromium } = require("playwright");
  const browser = await chromium.launch({ headless: true });
  t.after(() => browser.close());
  const page = await browser.newPage();
  await page.setContent(`
    <style>.reply { display: none } .message:hover .reply { display: inline }</style>
    <section class="message"><div><span>目标消息</span></div><button class="reply">回复</button></section>
  `);
  const locator = await locateMessageByText(page, "目标消息");
  assert.ok(locator);
  assert.match(await locator.innerText(), /目标消息/);
  assert.equal(await locator.locator("text=回复").first().isVisible(), true);
});
