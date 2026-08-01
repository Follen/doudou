"use strict";

const assert = require("node:assert/strict");
const fsp = require("node:fs/promises");
const os = require("node:os");
const path = require("node:path");
const test = require("node:test");
const { atomicWriteFile, readJsonl } = require("../src/core");
const { createRun, listInterruptedRuns, mergeMessages, readGroupsIndex, writeGroupsIndex } = require("../src/storage");

async function temporaryDirectory(t) {
  const directory = await fsp.mkdtemp(path.join(os.tmpdir(), "doudou-storage-"));
  t.after(() => fsp.rm(directory, { recursive: true, force: true }));
  return directory;
}

function message(overrides = {}) {
  return {
    schema: "doudou.message.v1",
    profile_id: "work",
    conversation_id: "conversation-123",
    conversation_name: "产品讨论",
    message_id: "message-1",
    sent_at: "2026-07-30T00:00:00.000Z",
    sender: { id: "sender-1", name: "Alice" },
    type: "text",
    text: "hello",
    images: [],
    reply_to_message_id: null,
    collected_at: "2026-07-30T01:00:00.000Z",
    run_id: "run-1",
    source: "douyin-web",
    parse_warnings: [],
    ...overrides,
  };
}

test("group index preserves discovery order and stable folder identity", async (t) => {
  const root = await temporaryDirectory(t);
  const index = await writeGroupsIndex(root, "work", [
    { conversation_id: "conversation-2", display_name: "同名群" },
    { conversation_id: "conversation-1", display_name: "同名群" },
  ]);
  assert.equal(index.count, 2);
  assert.deepEqual(index.groups.map((group) => group.conversation_id), ["conversation-2", "conversation-1"]);
  assert.notEqual(index.groups[0].folder_name, index.groups[1].folder_name);
  assert.deepEqual(await readGroupsIndex(root, "work"), index);
});

test("message merge deduplicates, fills missing values and audits conflicts", async (t) => {
  const root = await temporaryDirectory(t);
  const group = { conversation_id: "conversation-123", display_name: "产品讨论" };
  const first = await mergeMessages(root, "work", group, [message({ sender: { id: null, name: "Alice" } })], "run-1");
  assert.equal(first.added, 1);
  const second = await mergeMessages(root, "work", group, [message({ sender: { id: "sender-1", name: "Alice" }, run_id: "run-2" })], "run-2");
  assert.equal(second.total, 1);
  assert.equal(second.conflicts, 0);
  const merged = await readJsonl(second.messagesPath);
  assert.equal(merged[0].sender.id, "sender-1");
  const third = await mergeMessages(root, "work", group, [message({ text: "changed", run_id: "run-3" })], "run-3");
  assert.equal(third.total, 1);
  assert.ok(third.conflicts >= 1);
  const conflicts = await readJsonl(path.join(third.directory, "conflicts.jsonl"));
  assert.ok(conflicts.some((conflict) => conflict.field === "text"));
});

test("message merge upgrades unsupported types and empty image lists", async (t) => {
  const root = await temporaryDirectory(t);
  const group = { conversation_id: "conversation-123", display_name: "产品讨论" };
  await mergeMessages(root, "work", group, [message({ type: "unsupported", text: null, images: [] })], "run-1");
  const image = { url: "https://example.test/image.jpg", source: "structured", may_expire: false };
  const result = await mergeMessages(root, "work", group, [message({ type: "image", text: null, images: [image], run_id: "run-2" })], "run-2");
  assert.equal(result.conflicts, 0);
  const [merged] = await readJsonl(result.messagesPath);
  assert.equal(merged.type, "image");
  assert.deepEqual(merged.images, [image]);
});

test("downloaded media metadata replaces expiring URL-only image records", async (t) => {
  const root = await fsp.mkdtemp(path.join(os.tmpdir(), "doudou-media-merge-"));
  t.after(() => fsp.rm(root, { recursive: true, force: true }));
  const group = { conversation_id: "c1", display_name: "群", kind: "group", conversation_type: 2 };
  await mergeMessages(root, "work", group, [message({ images: [{ url: "https://example.test/image?token=old" }], protocol: { message_type: 27 } })], "run-1");
  const downloaded = { status: "downloaded", local_path: "message/image.png", format: "png", bytes: 10, md5: "m", sha256: "s" };
  const result = await mergeMessages(root, "work", group, [message({ images: [downloaded], protocol: { message_type: 27 }, run_id: "run-2" })], "run-2");
  const stored = JSON.parse((await fsp.readFile(result.messagesPath, "utf8")).trim());
  assert.deepEqual(stored.images, [downloaded]);
});

test("protocol-backed message types replace earlier generic classifications", async (t) => {
  const root = await temporaryDirectory(t);
  const group = { conversation_id: "conversation-123", display_name: "产品讨论" };
  await mergeMessages(root, "work", group, [message({ type: "text", text: "joined" })], "run-1");
  const result = await mergeMessages(root, "work", group, [message({ type: "system", text: "joined", protocol: { message_type: 1001 }, run_id: "run-2" })], "run-2");
  assert.equal(result.conflicts, 0);
  const [merged] = await readJsonl(result.messagesPath);
  assert.equal(merged.type, "system");
});

test("stored protocol metadata repairs stale top-level message types", async (t) => {
  const root = await temporaryDirectory(t);
  const group = { conversation_id: "conversation-123", display_name: "产品讨论" };
  await mergeMessages(root, "work", group, [message({ type: "unsupported", text: null, protocol: { message_type: 50001 } })], "run-1");
  const result = await mergeMessages(root, "work", group, [], "run-2");
  const [merged] = await readJsonl(result.messagesPath);
  assert.equal(merged.type, "system");
});

test("new protocol parsing replaces stale field mappings without conflicts", async (t) => {
  const root = await temporaryDirectory(t);
  const group = { conversation_id: "conversation-123", display_name: "产品讨论" };
  await mergeMessages(root, "work", group, [message({
    protocol: { message_type: 7, message_conv_index_v2: 12, unknown_fields: [{ field: 18, wire_type: 2 }] },
  })], "run-1");
  const result = await mergeMessages(root, "work", group, [message({
    protocol: { message_type: 7, index_in_conversation_v2: 12, reference_info: { referenced_message_id: "target" }, unknown_fields: [] },
    run_id: "run-2",
  })], "run-2");
  assert.equal(result.conflicts, 0);
  const [merged] = await readJsonl(result.messagesPath);
  assert.equal(merged.protocol.message_conv_index_v2, undefined);
  assert.equal(merged.protocol.index_in_conversation_v2, 12);
  assert.equal(merged.protocol.reference_info.referenced_message_id, "target");
  assert.deepEqual(merged.protocol.unknown_fields, []);
});

test("collection Run starts as running with fixed group entries", async (t) => {
  const root = await temporaryDirectory(t);
  const { run, filename } = await createRun(root, "work", ["a", "b"], "2026-01-01T00:00:00.000Z", "2026-01-02T00:00:00.000Z");
  assert.equal(run.status, "running");
  assert.equal(run.groups.length, 2);
  assert.match(filename, /runs[\\/]run-/);
  const interrupted = await listInterruptedRuns(root, "work");
  assert.equal(interrupted.length, 1);
  assert.equal(interrupted[0].run_id, run.run_id);
});

test("unknown message schemas stop JSONL reads with a stable error", async (t) => {
  const root = await temporaryDirectory(t);
  const filename = path.join(root, "messages.jsonl");
  await atomicWriteFile(filename, '{"schema":"doudou.message.v9"}\n');
  const { validateRecord } = require("../src/schemas");
  await assert.rejects(() => readJsonl(filename, { validate: validateRecord }), (error) => error.code === "UNKNOWN_SCHEMA");
});
