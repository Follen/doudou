import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import test from "node:test";

import { currentRuntime, doudouCacheRoot } from "../lib/python.mjs";
import {
  expectedManagedRuntime,
  managedPython,
  managedRuntimeReady,
  platformAssetKey,
} from "../lib/runtime-setup.mjs";

function fixture(t) {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "doudou-runtime-test-"));
  t.after(() => fs.rmSync(root, { recursive: true, force: true }));
  const skill = path.join(root, "skill");
  fs.mkdirSync(path.join(skill, "scripts"), { recursive: true });
  fs.writeFileSync(
    path.join(skill, "scripts", "runtime-manifest.json"),
    JSON.stringify({
      schema: "doudou.runtime-manifest.v2",
      runtime_contract: "test",
      python: { version: "3.12.13", assets: { "linux-x64": { url: "https://example.invalid/python", size: 1, sha256: "a" } } },
      camoufox_package: "0.5.4",
      camoufox_browser: "official/stable/152.0.4-beta.28",
      camoufox_assets: { "linux-x64": { url: "https://example.invalid/browser", size: 1, sha256: "b" } },
      download: {},
    }),
  );
  return { root, skill, env: { DOUDOU_HOME: path.join(root, ".doudou") } };
}

test("managed runtime never probes system Python", (t) => {
  const { root, skill, env } = fixture(t);
  const expected = expectedManagedRuntime({ skillDirectory: skill, platform: "linux", arch: "x64", env, home: root });
  assert.match(expected.runtimeId, /^py312-test-/);
  assert.equal(expected.root, path.join(env.DOUDOU_HOME, "runtimes"));
  assert.equal(platformAssetKey("linux", "x64"), "linux-x64");
  assert.throws(() => platformAssetKey("darwin", "arm64"), /does not publish/);
});

test("runtime readiness requires a verified pointer and nested venv", (t) => {
  const { root, skill, env } = fixture(t);
  const options = { skillDirectory: skill, platform: "linux", arch: "x64", env, home: root };
  const expected = expectedManagedRuntime(options);
  const executable = managedPython(expected.directory, "linux");
  fs.mkdirSync(path.dirname(executable), { recursive: true });
  fs.writeFileSync(executable, "", "utf8");
  fs.writeFileSync(path.join(expected.directory, "runtime.json"), JSON.stringify({ status: "verified" }));
  fs.mkdirSync(path.dirname(expected.pointer), { recursive: true });
  fs.writeFileSync(expected.pointer, JSON.stringify({ runtime_id: expected.runtimeId, path: expected.directory }));
  assert.equal(managedRuntimeReady({ ...options, probeRuntime: false }), true);
  assert.equal(currentRuntime({ platform: "linux", env, home: root }).executable, executable);
});

test("Windows readiness checks Scripts/python.exe instead of the Linux path", (t) => {
  const { root, skill } = fixture(t);
  const env = { DOUDOU_HOME: path.join(root, ".doudou") };
  const options = { skillDirectory: skill, platform: "win32", arch: "x64", env, home: root };
  const expected = expectedManagedRuntime(options);
  const executable = managedPython(expected.directory, "win32");
  fs.mkdirSync(path.dirname(executable), { recursive: true });
  fs.writeFileSync(executable, "", "utf8");
  fs.writeFileSync(path.join(expected.directory, "runtime.json"), JSON.stringify({ status: "verified" }));
  fs.mkdirSync(path.dirname(expected.pointer), { recursive: true });
  fs.writeFileSync(expected.pointer, JSON.stringify({ runtime_id: expected.runtimeId, path: expected.directory }));
  assert.equal(managedRuntimeReady({ ...options, probeRuntime: false }), true);
  assert.equal(currentRuntime({ platform: "win32", env, home: root }).executable, executable);
  assert.equal(fs.existsSync(path.join(expected.directory, "venv", "bin", "python")), false);
});

test("DOUDOU_HOME defaults to the user home and supports an override", () => {
  assert.equal(doudouCacheRoot({ platform: "linux", env: {}, home: "/home/user" }), path.join("/home/user", ".doudou"));
  assert.equal(doudouCacheRoot({ platform: "linux", env: { DOUDOU_HOME: "/managed/doudou" }, home: "/home/user" }), path.resolve("/managed/doudou"));
  assert.equal(doudouCacheRoot({ platform: "linux", env: { DOUDOU_HOME: "custom" }, home: "/home/user" }), path.resolve("/home/user", "custom"));
});
