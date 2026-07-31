"use strict";

const assert = require("node:assert/strict");
const fsp = require("node:fs/promises");
const os = require("node:os");
const path = require("node:path");
const test = require("node:test");
const { atomicWriteJsonl, parseArgs, readJsonl, safeSegment } = require("../src/core");

async function temporaryDirectory(t) {
  const directory = await fsp.mkdtemp(path.join(os.tmpdir(), "douyin-group-message-"));
  t.after(() => fsp.rm(directory, { recursive: true, force: true }));
  return directory;
}

test("parseArgs handles positionals, values, equals and booleans", () => {
  assert.deepEqual(parseArgs(["collect", "--profile", "工作", "--groups=a,b", "--terminate-owner"]), {
    _: ["collect"],
    profile: "工作",
    groups: "a,b",
    terminate_owner: true,
  });
});

test("safeSegment encodes forbidden path characters deterministically", () => {
  assert.equal(safeSegment("产品/讨论"), "产品~002F讨论");
  assert.equal(safeSegment(".."), "~2e2e");
  assert.equal(safeSegment("a:b"), "a~003Ab");
});

test("JSONL writes are complete and incomplete final lines fail", async (t) => {
  const directory = await temporaryDirectory(t);
  const filename = path.join(directory, "records.jsonl");
  await atomicWriteJsonl(filename, [{ a: 1 }, { a: 2 }]);
  assert.deepEqual(await readJsonl(filename), [{ a: 1 }, { a: 2 }]);
  await fsp.writeFile(filename, '{"a":1}', "utf8");
  await assert.rejects(() => readJsonl(filename), (error) => error.code === "INCOMPLETE_JSONL_LINE");
});
