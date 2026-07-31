"use strict";

const assert = require("node:assert/strict");
const test = require("node:test");
const { defaultMessageWindow, normalizeLookupText, resolveConversationTarget, resolveMessageTarget } = require("../src/resolver");

const index = {
  conversations: [
    { conversation_id: "g1", conversation_short_id: "11", conversation_type: 2, kind: "group", display_name: "萌新 18群" },
    { conversation_id: "g2", conversation_short_id: "12", conversation_type: 2, kind: "group", display_name: "萌新 18群（顶峰天赋）" },
    { conversation_id: "d1", conversation_short_id: "13", conversation_type: 1, kind: "direct", display_name: "Alice Smith" },
  ],
};

test("conversation targets support normalized ids and unique name substrings", () => {
  assert.equal(resolveConversationTarget(index, "d1").matched_by, "id");
  const resolved = resolveConversationTarget(index, "alice   SMITH");
  assert.equal(resolved.conversation.conversation_id, "d1");
  assert.equal(resolved.matched_by, "name_contains");
});

test("conversation targets return every substring match even with an exact name", () => {
  assert.throws(() => resolveConversationTarget(index, "萌新 18群"), (error) => {
    assert.equal(error.code, "CONVERSATION_AMBIGUOUS");
    assert.deepEqual(error.details.candidates.map((item) => item.conversation_id), ["g1", "g2"]);
    assert.deepEqual(error.details.candidates.map((item) => item.number), [1, 2]);
    return true;
  });
  assert.throws(() => resolveConversationTarget(index, "不存在"), (error) => error.code === "RECIPIENT_NOT_FOUND");
});

test("message targets collapse whitespace and ignore case while retaining stable candidates", () => {
  const messages = [
    { message_id: "m1", sender: { name: "A" }, sent_at: "2026-07-31T01:00:00.000Z", type: "text", text: "Hello\nWorld" },
    { message_id: "m2", sender: { name: "B" }, sent_at: "2026-07-31T02:00:00.000Z", type: "text", text: "prefix hello world suffix" },
  ];
  assert.equal(normalizeLookupText("  HELLO\tworld "), "hello world");
  assert.throws(() => resolveMessageTarget(messages, "hello   WORLD"), (error) => {
    assert.equal(error.code, "MESSAGE_AMBIGUOUS");
    assert.deepEqual(error.details.candidates.map((item) => item.message_id), ["m1", "m2"]);
    return true;
  });
  assert.equal(resolveMessageTarget(messages, "prefix hello").message.message_id, "m2");
  assert.throws(() => resolveMessageTarget(messages, "missing"), (error) => error.code === "MESSAGE_NOT_FOUND");
});

test("default message window spans local midnight through command time", () => {
  const clock = new Date(2026, 6, 31, 13, 45, 12, 345);
  const window = defaultMessageWindow(clock);
  const expectedStart = new Date(clock);
  expectedStart.setHours(0, 0, 0, 0);
  assert.deepEqual(window, { from: expectedStart.toISOString(), to: clock.toISOString(), defaulted: true });
});
