"use strict";

const assert = require("node:assert/strict");
const fsp = require("node:fs/promises");
const os = require("node:os");
const path = require("node:path");
const test = require("node:test");
const { readLatestWarmup, writeWarmupSnapshot } = require("../src/storage");
const { WARMUP_CACHE_MS, readFreshWarmup, summarizeConversations } = require("../src/warmup");

function snapshot(completedAt, overrides = {}) {
  return {
    schema: "douyin-message.warmup.v1",
    profile_id: "work",
    completed_at: completedAt,
    status: "success",
    conversations_index: { conversations: [] },
    ...overrides,
  };
}

test("warmup cache accepts successful snapshots through two hours only", () => {
  const clock = Date.parse("2026-07-31T12:00:00.000Z");
  assert.equal(readFreshWarmup(snapshot(new Date(clock - WARMUP_CACHE_MS).toISOString()), clock).age_ms, WARMUP_CACHE_MS);
  assert.equal(readFreshWarmup(snapshot(new Date(clock - WARMUP_CACHE_MS - 1).toISOString()), clock), null);
  assert.equal(readFreshWarmup(snapshot(new Date(clock + 1).toISOString()), clock), null);
  assert.equal(readFreshWarmup(snapshot(new Date(clock - 1000).toISOString(), { status: "failed" }), clock), null);
});

test("warmup snapshots write dated history and latest pointers", async (t) => {
  const root = await fsp.mkdtemp(path.join(os.tmpdir(), "douyin-warmup-"));
  t.after(() => fsp.rm(root, { recursive: true, force: true }));
  const value = snapshot("2026-07-31T12:34:56.789Z");
  const files = await writeWarmupSnapshot(root, "work", value);
  assert.equal(JSON.parse(await fsp.readFile(files.filename, "utf8")).completed_at, value.completed_at);
  assert.deepEqual(await readLatestWarmup(root, "work"), value);
  assert.match(files.filename, /warmup[\\/]2026-07-31[\\/]20260731T123456789Z__warmup\.json$/);
});

test("warmup summaries keep direct and group counts separate", () => {
  const summary = summarizeConversations({ count: 3, conversations: [
    { kind: "direct", discovery_status: "active" },
    { kind: "group", discovery_status: "active" },
    { kind: "group", discovery_status: "unresolved_id" },
  ] });
  assert.deepEqual(summary, { count: 3, direct_count: 1, group_count: 2, active_count: 2, unresolved_count: 1 });
});
