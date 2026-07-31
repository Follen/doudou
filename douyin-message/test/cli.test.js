"use strict";

const assert = require("node:assert/strict");
const childProcess = require("node:child_process");
const fsp = require("node:fs/promises");
const os = require("node:os");
const path = require("node:path");
const test = require("node:test");
const { mergeMessages, writeConversationsIndex, writeGroupsIndex, writeWarmupSnapshot } = require("../src/storage");

const cli = path.resolve(__dirname, "..", "bin", "douyin-message.js");

async function seedWarmup(root, profileId, conversations) {
  const index = await writeConversationsIndex(root, profileId, conversations);
  await writeWarmupSnapshot(root, profileId, {
    schema: "douyin-message.warmup.v1",
    profile_id: profileId,
    started_at: new Date(Date.now() - 1000).toISOString(),
    completed_at: new Date().toISOString(),
    status: "success",
    headless: true,
    fingerprint_captured_at: null,
    conversations: { count: index.count, direct_count: 0, group_count: 0, active_count: index.count, unresolved_count: 0 },
    conversations_index: index,
    contract: { dom: {}, network: { json_response_count: 0, protobuf_response_count: 0, skipped_response_count: 0, endpoints: [] } },
  });
  return index;
}

test("CLI help documents headed login and headless task behavior", () => {
  const result = childProcess.spawnSync(process.execPath, [cli, "help"], { encoding: "utf8" });
  assert.equal(result.status, 0);
  assert.match(result.stdout, /fingerprint calibration and manual verification are headed/);
  assert.match(result.stdout, /persisted Profile fingerprint/);
  assert.match(result.stdout, /--auto-calibrate-fallback/);
  assert.match(result.stdout, /profile calibrate --profile NAME/);
});

test("doctor initializes only project profile and message roots from an installed Skill", async (t) => {
  const root = await fsp.mkdtemp(path.join(os.tmpdir(), "douyin-project-"));
  t.after(() => fsp.rm(root, { recursive: true, force: true }));
  const result = childProcess.spawnSync(process.execPath, [cli, "doctor", "--skip-launch"], { cwd: root, encoding: "utf8" });
  assert.equal(result.status, 0, result.stdout + result.stderr);
  const events = result.stdout.trim().split(/\r?\n/).map(JSON.parse);
  const started = events[0];
  const doctor = events.at(-1).result;
  assert.equal(started.data_root, root);
  assert.equal(doctor.node.supported, true);
  assert.equal(doctor.npm.available, true);
  assert.equal(doctor.playwright.matches, true);
  assert.equal(doctor.chromium.exists, true);
  assert.equal(doctor.filesystem.profiles.path, path.join(root, "profile"));
  assert.equal(doctor.filesystem.message.path, path.join(root, "message"));
  assert.equal(await fsp.stat(path.join(root, "profile")).then((item) => item.isDirectory()), true);
  assert.equal(await fsp.stat(path.join(root, "message")).then((item) => item.isDirectory()), true);
  await assert.rejects(fsp.stat(path.join(root, "analyze")), (error) => error.code === "ENOENT");
});

test("CLI creates and lists a Profile with machine-readable output", async (t) => {
  const root = await fsp.mkdtemp(path.join(os.tmpdir(), "douyin-cli-"));
  t.after(() => fsp.rm(root, { recursive: true, force: true }));
  const created = childProcess.spawnSync(process.execPath, [cli, "profile", "create", "--project-root", root, "--profile", "work"], { encoding: "utf8" });
  assert.equal(created.status, 0, created.stdout + created.stderr);
  assert.equal(await fsp.stat(path.join(root, "profile", "work", "browser-data")).then((item) => item.isDirectory()), true);
  const listed = childProcess.spawnSync(process.execPath, [cli, "profile", "list", "--project-root", root], { encoding: "utf8" });
  assert.equal(listed.status, 0, listed.stdout + listed.stderr);
  const events = listed.stdout.trim().split(/\r?\n/).map(JSON.parse);
  assert.equal(events.at(-1).result.count, 1);
  assert.equal(events.at(-1).result.profiles[0].profile_id, "work");
});

test("CLI resolves numbered group selections only against the shown refresh", async (t) => {
  const root = await fsp.mkdtemp(path.join(os.tmpdir(), "douyin-select-"));
  t.after(() => fsp.rm(root, { recursive: true, force: true }));
  const index = await writeGroupsIndex(root, "work", [
    { conversation_id: "c1", display_name: "一群" },
    { conversation_id: "c2", display_name: "二群" },
    { conversation_id: "c3", display_name: "三群" },
  ]);
  const selected = childProcess.spawnSync(process.execPath, [cli, "groups", "select", "--project-root", root, "--profile", "work", "--refreshed-at", index.refreshed_at, "--selection", "1,3"], { encoding: "utf8" });
  assert.equal(selected.status, 0, selected.stdout + selected.stderr);
  const result = selected.stdout.trim().split(/\r?\n/).map(JSON.parse).at(-1).result;
  assert.deepEqual(result.groups.map((group) => group.conversation_id), ["c1", "c3"]);
  const stale = childProcess.spawnSync(process.execPath, [cli, "groups", "select", "--project-root", root, "--profile", "work", "--refreshed-at", "2000-01-01T00:00:00.000Z", "--selection", "1"], { encoding: "utf8" });
  assert.equal(stale.status, 1);
  assert.equal(JSON.parse(stale.stdout.trim().split(/\r?\n/).at(-1)).code, "GROUP_SELECTION_STALE");
});

test("CLI selects direct conversations and previews protocol sends", async (t) => {
  const root = await fsp.mkdtemp(path.join(os.tmpdir(), "douyin-direct-"));
  t.after(() => fsp.rm(root, { recursive: true, force: true }));
  const index = await seedWarmup(root, "work", [{
    conversation_id: "direct-1",
    conversation_short_id: "101",
    conversation_type: 1,
    kind: "direct",
    display_name: "私聊用户",
  }]);
  assert.equal(await fsp.stat(path.join(root, "message", "profiles", "work", "conversations.json")).then((item) => item.isFile()), true);
  await assert.rejects(fsp.stat(path.join(root, "analyze")), (error) => error.code === "ENOENT");
  const selected = childProcess.spawnSync(process.execPath, [cli, "conversations", "select", "--data-root", root, "--profile", "work", "--refreshed-at", index.refreshed_at, "--selection", "1"], { encoding: "utf8" });
  assert.equal(selected.status, 0, selected.stdout + selected.stderr);
  const selection = selected.stdout.trim().split(/\r?\n/).map(JSON.parse).at(-1).result;
  assert.equal(selection.conversations[0].kind, "direct");
  const previewed = childProcess.spawnSync(process.execPath, [cli, "protocol", "send", "--data-root", root, "--profile", "work", "--conversation-id", "direct-1", "--text", "hello"], { encoding: "utf8" });
  assert.equal(previewed.status, 0, previewed.stdout + previewed.stderr);
  const preview = previewed.stdout.trim().split(/\r?\n/).map(JSON.parse).at(-1).result;
  assert.equal(preview.status, "confirmation_required");
  assert.equal(preview.preview.conversation_type, 1);
  assert.equal(preview.preview.kind, "direct");
  assert.equal(preview.preview.warmup_source, "stale_cache");
  const missing = childProcess.spawnSync(process.execPath, [cli, "protocol", "send", "--data-root", root, "--profile", "work", "--recipient", "不存在", "--text", "hello"], { encoding: "utf8" });
  assert.equal(missing.status, 1);
  assert.equal(JSON.parse(missing.stdout.trim().split(/\r?\n/).at(-1)).code, "RECIPIENT_NOT_FOUND");
  const mismatched = childProcess.spawnSync(process.execPath, [cli, "protocol", "send", "--data-root", root, "--profile", "work", "--conversation-id", "direct-1", "--text", "hello", "--confirm", "--plan-id", "wrong"], { encoding: "utf8" });
  assert.equal(mismatched.status, 1);
  assert.equal(JSON.parse(mismatched.stdout.trim().split(/\r?\n/).at(-1)).code, "PROTOCOL_PLAN_MISMATCH");
  const freshRequired = childProcess.spawnSync(process.execPath, [cli, "protocol", "send", "--data-root", root, "--profile", "work", "--conversation-id", "direct-1", "--text", "hello", "--confirm", "--plan-id", "protocol-plan-0000000000000000"], { encoding: "utf8" });
  assert.equal(freshRequired.status, 1);
  assert.equal(JSON.parse(freshRequired.stdout.trim().split(/\r?\n/).at(-1)).code, "PROFILE_NOT_FOUND");
});

test("single reply first returns a preview and appends a planned audit event", async (t) => {
  const root = await fsp.mkdtemp(path.join(os.tmpdir(), "douyin-reply-"));
  t.after(() => fsp.rm(root, { recursive: true, force: true }));
  const group = { conversation_id: "c1", display_name: "测试群" };
  await writeGroupsIndex(root, "work", [group]);
  await seedWarmup(root, "work", [{ ...group, conversation_short_id: "c1", conversation_type: 2, kind: "group" }]);
  const stored = await mergeMessages(root, "work", group, [{
    schema: "douyin-group-message.message.v1",
    profile_id: "work",
    conversation_id: "c1",
    conversation_name: "测试群",
    message_id: "m1",
    sent_at: "2026-07-30T00:00:00.000Z",
    sender: { id: "u1", name: "User" },
    type: "text",
    text: "target",
    images: [],
    reply_to_message_id: null,
    collected_at: "2026-07-30T01:00:00.000Z",
    run_id: "run-1",
    source: "douyin-web",
    parse_warnings: [],
  }], "run-1");
  const preview = childProcess.spawnSync(process.execPath, [cli, "reply", "--project-root", root, "--profile", "work", "--conversation-id", "c1", "--message-id", "m1", "--text", "reply"], { encoding: "utf8" });
  assert.equal(preview.status, 0, preview.stdout + preview.stderr);
  const result = preview.stdout.trim().split(/\r?\n/).map(JSON.parse).at(-1).result;
  assert.equal(result.status, "confirmation_required");
  assert.deepEqual(result.retry_arguments.slice(0, 2), ["--confirm", "--plan-id"]);
  const audit = await fsp.readFile(path.join(stored.directory, "replies.jsonl"), "utf8");
  assert.equal(JSON.parse(audit.trim()).event_type, "planned");
});
