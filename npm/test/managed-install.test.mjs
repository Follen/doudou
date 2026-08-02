import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import test from "node:test";

import {
  managedInstallFile,
  readManagedInstall,
  recordManagedSkill,
} from "../lib/managed-install.mjs";

function temporaryDirectory(t) {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "doudou-managed-install-test-"));
  t.after(() => fs.rmSync(root, { recursive: true, force: true }));
  return root;
}

test("records every managed Skill target without duplicates", (t) => {
  const root = temporaryDirectory(t);
  const options = {
    platform: "linux",
    env: { DOUDOU_HOME: path.join(root, ".doudou") },
    home: root,
  };
  const first = path.join(root, "first");
  const second = path.join(root, "second");
  recordManagedSkill(first, options);
  recordManagedSkill(second, options);
  recordManagedSkill(first, options);
  const state = readManagedInstall(options);
  assert.deepEqual(
    state.targets.map((entry) => entry.path).sort(),
    [path.resolve(first), path.resolve(second)].sort(),
  );
  assert.equal(state.targets.length, 2);
  assert.ok(fs.existsSync(managedInstallFile(options)));
});

test("rejects a filesystem root as a managed Skill", (t) => {
  const root = temporaryDirectory(t);
  assert.throws(
    () =>
      recordManagedSkill(path.parse(root).root, {
        platform: "linux",
        env: { DOUDOU_HOME: path.join(root, ".doudou") },
        home: root,
      }),
    /filesystem root/,
  );
});
