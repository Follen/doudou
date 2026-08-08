import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

import { recordManagedSkill } from "../lib/managed-install.mjs";
import { doudouCacheRoot } from "../lib/python.mjs";
import { ensureSkill, INSTALL_STAMP } from "../lib/skill-installer.mjs";
import {
  cleanupManagedInstall,
  discoverManagedBrokerProcesses,
  ManagedUninstallError,
} from "../lib/uninstall.mjs";

const packageRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..", "..");

function temporaryDirectory(t) {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "doudou-uninstall-test-"));
  t.after(() => fs.rmSync(root, { recursive: true, force: true }));
  return root;
}

function options(root) {
  return {
    platform: "linux",
    env: { DOUDOU_HOME: path.join(root, "home", ".doudou") },
    home: path.join(root, "home"),
  };
}

async function installManagedSkill(target, managedOptions) {
  await ensureSkill({ packageRoot, target });
  recordManagedSkill(target, managedOptions);
}

test("removes all managed Skills and DOUDOU_HOME while preserving project data", async (t) => {
  const root = temporaryDirectory(t);
  const managedOptions = options(root);
  const first = path.join(root, "skills", "first");
  const second = path.join(root, "skills", "second");
  await installManagedSkill(first, managedOptions);
  await installManagedSkill(second, managedOptions);
  const cacheRoot = doudouCacheRoot(managedOptions);
  const runtime = path.join(cacheRoot, "runtimes", "runtime", "bin");
  const browser = path.join(cacheRoot, "browsers", "browser.bin");
  const externalBrowser = path.join(root, "shared-camoufox", "browser.bin");
  const profile = path.join(root, "project", "profile", "work", "profile.json");
  const message = path.join(root, "project", "message", "profiles", "work", "messages.jsonl");
  for (const filename of [path.join(runtime, "python"), browser, externalBrowser, profile, message]) {
    fs.mkdirSync(path.dirname(filename), { recursive: true });
    fs.writeFileSync(filename, filename, "utf8");
  }

  let stoppedRuntime;
  const result = await cleanupManagedInstall({
    ...managedOptions,
    stopBrokers: async (runtimeRoot) => {
      stoppedRuntime = runtimeRoot;
      return [];
    },
  });

  assert.equal(stoppedRuntime, path.join(cacheRoot, "runtimes"));
  assert.equal(fs.existsSync(first), false);
  assert.equal(fs.existsSync(second), false);
  assert.equal(fs.existsSync(cacheRoot), false);
  assert.equal(fs.readFileSync(externalBrowser, "utf8"), externalBrowser);
  assert.equal(fs.readFileSync(profile, "utf8"), profile);
  assert.equal(fs.readFileSync(message, "utf8"), message);
  assert.equal(result.browser_cache, "removed");
  assert.equal(result.project_data, "preserved");
  assert.deepEqual(
    result.skills.filter((entry) => entry.status === "removed").map((entry) => entry.target).sort(),
    [path.resolve(first), path.resolve(second)].sort(),
  );
});

test("preserves a registered directory whose ownership marker is missing", async (t) => {
  const root = temporaryDirectory(t);
  const managedOptions = options(root);
  const target = path.join(root, "unknown-skill");
  fs.mkdirSync(target, { recursive: true });
  fs.writeFileSync(path.join(target, "user.txt"), "keep", "utf8");
  recordManagedSkill(target, managedOptions);

  const result = await cleanupManagedInstall({
    ...managedOptions,
    stopBrokers: async () => [],
  });

  assert.equal(fs.readFileSync(path.join(target, "user.txt"), "utf8"), "keep");
  assert.equal(
    result.skills.find((entry) => entry.target === path.resolve(target)).status,
    "preserved_unmanaged",
  );
});

test("fails without clearing ownership state when a managed Skill cannot be removed", async (t) => {
  const root = temporaryDirectory(t);
  const managedOptions = options(root);
  const target = path.join(root, "managed-skill");
  await installManagedSkill(target, managedOptions);

  await assert.rejects(
    cleanupManagedInstall({
      ...managedOptions,
      stopBrokers: async () => [],
      removePath(candidate) {
        if (path.resolve(candidate) === path.resolve(target)) {
          throw new Error("locked");
        }
        fs.rmSync(candidate, { recursive: true, force: true });
      },
    }),
    (error) => {
      assert.ok(error instanceof ManagedUninstallError);
      assert.equal(error.code, "DOUDOU_UNINSTALL_FAILED");
      assert.equal(error.details.object, path.resolve(target));
      return true;
    },
  );
  assert.ok(fs.existsSync(target));
});

test("managed Skill stamp binds the marker to its installed target", async (t) => {
  const root = temporaryDirectory(t);
  const target = path.join(root, "skill");
  await ensureSkill({ packageRoot, target });
  const stamp = JSON.parse(fs.readFileSync(path.join(target, INSTALL_STAMP), "utf8"));
  assert.equal(stamp.target, path.resolve(target));
});

test("Windows broker discovery requires both runtime ancestry and broker module", (t) => {
  const root = temporaryDirectory(t);
  const runtime = path.join(root, "runtimes");
  fs.mkdirSync(runtime, { recursive: true });
  const records = discoverManagedBrokerProcesses(runtime, {
    platform: "win32",
    run: () => ({
      status: 0,
      stdout: JSON.stringify([
        {
          ProcessId: 10,
          ExecutablePath: path.join(runtime, "one", "python.exe"),
          CommandLine: "python -m doudou.features.broker --profile work",
        },
        {
          ProcessId: 11,
          ExecutablePath: path.join(runtime, "one", "python.exe"),
          CommandLine: "python other.py",
        },
        {
          ProcessId: 12,
          ExecutablePath: path.join(root, "outside", "python.exe"),
          CommandLine: "python -m doudou.features.broker --profile work",
        },
      ]),
    }),
  });
  assert.deepEqual(records.map((record) => record.pid), [10]);
});
