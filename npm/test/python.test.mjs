import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import test from "node:test";

import {
  expectedRuntime,
  pythonCandidates,
  runtimeReady,
  supportedPythonVersion,
} from "../lib/python.mjs";

test("Python candidate order is stable per platform", () => {
  assert.deepEqual(
    pythonCandidates("win32").map((item) => [item.command, item.prefix]),
    [
      ["py", ["-3"]],
      ["python3", []],
      ["python", []],
    ],
  );
  assert.deepEqual(
    pythonCandidates("linux").map((item) => item.command),
    ["python3", "python"],
  );
});

test("supported Python range is >=3.10,<4", () => {
  assert.equal(supportedPythonVersion({ major: 3, minor: 10 }), true);
  assert.equal(supportedPythonVersion({ major: 3, minor: 13 }), true);
  assert.equal(supportedPythonVersion({ major: 3, minor: 9 }), false);
  assert.equal(supportedPythonVersion({ major: 4, minor: 0 }), false);
});

test("runtime readiness follows the versioned current pointer", (t) => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "doudou-python-test-"));
  t.after(() => fs.rmSync(root, { recursive: true, force: true }));
  const skill = path.join(root, "skill");
  fs.mkdirSync(path.join(skill, "scripts"), { recursive: true });
  fs.writeFileSync(
    path.join(skill, "scripts", "runtime-manifest.json"),
    JSON.stringify({
      camoufox_package: "0.5.4",
      camoufox_browser: "official/stable/152.0.4-beta.28",
    }),
  );
  const env = { XDG_CACHE_HOME: path.join(root, "cache") };
  const python = { version: { major: 3, minor: 12 } };
  const expected = expectedRuntime({
    skillDirectory: skill,
    python,
    platform: "linux",
    env,
    home: root,
  });
  const runtime = path.join(root, "runtime");
  fs.mkdirSync(path.join(runtime, "bin"), { recursive: true });
  fs.writeFileSync(path.join(runtime, "bin", "python"), "", "utf8");
  fs.mkdirSync(path.dirname(expected.pointer), { recursive: true });
  fs.writeFileSync(
    expected.pointer,
    JSON.stringify({ runtime_id: expected.runtimeId, path: runtime }),
  );
  assert.equal(
    runtimeReady({
      skillDirectory: skill,
      python,
      platform: "linux",
      env,
      home: root,
    }),
    true,
  );
});
