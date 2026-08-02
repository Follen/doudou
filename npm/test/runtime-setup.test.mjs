import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { spawn } from "node:child_process";
import test from "node:test";

import { DownloadError } from "../lib/controlled-download.mjs";
import { setupDoctor, setupInternals } from "../lib/runtime-setup.mjs";

function fixtureSkill(root) {
  const skill = path.join(root, "skill");
  fs.mkdirSync(path.join(skill, "scripts"), { recursive: true });
  fs.copyFileSync(
    new URL("../../doudou/scripts/runtime-manifest.json", import.meta.url),
    path.join(skill, "scripts", "runtime-manifest.json"),
  );
  return skill;
}

test("concurrent setup callers share one owner", async (t) => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "doudou-setup-lock-"));
  t.after(() => fs.rmSync(root, { recursive: true, force: true }));
  const first = await setupInternals.acquireSetupLock(root, {});
  let waiting = false;
  const secondPromise = setupInternals.acquireSetupLock(root, {
    lockPollMs: 10,
    onProgress(event) {
      if (event.stage === "waiting" && event.owner_pid === process.pid) waiting = true;
    },
  });
  await new Promise((resolve) => setTimeout(resolve, 30));
  assert.equal(waiting, true);
  setupInternals.releaseSetupLock(first);
  const second = await secondPromise;
  assert.equal(second.owned, true);
  assert.notEqual(second.owner.token, first.owner.token);
  setupInternals.releaseSetupLock(second);
});

test("a stale setup lock is reclaimed", async (t) => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "doudou-setup-stale-"));
  t.after(() => fs.rmSync(root, { recursive: true, force: true }));
  fs.writeFileSync(
    path.join(root, "setup.lock"),
    JSON.stringify({ token: "stale", pid: 2147483647 }),
  );
  const owner = await setupInternals.acquireSetupLock(root, {});
  assert.equal(owner.owned, true);
  assert.notEqual(owner.owner.token, "stale");
  setupInternals.releaseSetupLock(owner);
});

test("stage changes clear completed download state", async (t) => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "doudou-setup-state-"));
  t.after(() => fs.rmSync(root, { recursive: true, force: true }));
  const lock = await setupInternals.acquireSetupLock(root, {});
  setupInternals.writeOwner(
    lock.paths,
    lock.owner,
    { stage: "download", downloaded: 10, total: 10, completed: true, heartbeat: true },
    {},
  );
  setupInternals.writeOwner(lock.paths, lock.owner, { stage: "extract" }, {});
  const state = JSON.parse(fs.readFileSync(lock.paths.state, "utf8"));
  assert.equal(state.completed, undefined);
  assert.equal(state.heartbeat, undefined);
  assert.equal(state.downloaded, undefined);
  assert.equal(state.total, undefined);
  setupInternals.releaseSetupLock(lock);
});

test("doctor reports setup-required and a live owner", async (t) => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "doudou-setup-doctor-"));
  t.after(() => fs.rmSync(root, { recursive: true, force: true }));
  const skillDirectory = fixtureSkill(root);
  fs.mkdirSync(path.join(root, "doudou"), { recursive: true });
  fs.writeFileSync(
    path.join(root, "doudou", "setup-state.json"),
    JSON.stringify({ pid: process.pid, stage: "dependencies" }),
  );
  const result = setupDoctor({
    skillDirectory,
    platform: "linux",
    arch: "x64",
    env: { DOUDOU_HOME: path.join(root, "doudou") },
    home: root,
    probeRuntime: false,
  });
  assert.equal(result.status, "setup_required");
  assert.equal(result.ready, false);
  assert.equal(result.owner_alive, true);
  assert.equal(result.diagnosis, "setup_in_progress");
  assert.equal(result.browser_cache.valid, false);
  assert.equal(result.repair_command, "doudou setup");
});

test("browser download failures use the public browser error namespace", () => {
  const source = new DownloadError("DOWNLOAD_CHECKSUM_MISMATCH", "bad digest", { expected: "fixture" });
  const error = setupInternals.scopedDownloadError(source, "BROWSER");
  assert.equal(error.code, "BROWSER_DOWNLOAD_CHECKSUM_MISMATCH");
  assert.equal(error.details.expected, "fixture");
});

test("Camoufox assets only use the dedicated DOUDOU_HOME browser root", () => {
  const home = path.join(os.tmpdir(), "doudou-home-fixture");
  const result = setupInternals.browserCacheRoot({
    platform: "win32",
    env: {
      DOUDOU_HOME: home,
      DOUDOU_CAMOUFOX_CACHE: path.join(os.tmpdir(), "shared-cache"),
      LOCALAPPDATA: path.join(os.tmpdir(), "local-app-data"),
    },
    home: os.homedir(),
  });
  assert.equal(result, path.join(home, "browsers"));
});

test("a verified DOUDOU_HOME removes only a recognized legacy runtime root", (t) => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "doudou-legacy-runtime-"));
  t.after(() => fs.rmSync(root, { recursive: true, force: true }));
  const current = path.join(root, "home", ".doudou");
  const legacy = path.join(root, "legacy-cache", "doudou");
  fs.mkdirSync(path.join(legacy, "runtimes"), { recursive: true });
  fs.writeFileSync(path.join(legacy, "runtimes", "current.json"), "{}");
  const result = setupInternals.cleanupLegacyInstall(
    {
      platform: "linux",
      env: { DOUDOU_HOME: current, XDG_CACHE_HOME: path.join(root, "legacy-cache") },
      home: path.join(root, "home"),
    },
    current,
  );
  assert.equal(result.status, "removed");
  assert.equal(fs.existsSync(legacy), false);
  assert.equal(fs.existsSync(path.join(current, "migrations", "legacy-cache-v1.json")), true);
});

test("interrupted runtime staging is removed and the previous runtime restored", (t) => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "doudou-setup-recover-"));
  t.after(() => fs.rmSync(root, { recursive: true, force: true }));
  const directory = path.join(root, "runtime-a");
  const replaced = `${directory}.replaced-1`;
  const temporary = path.join(root, ".runtime-a.install-fixture");
  fs.mkdirSync(replaced, { recursive: true });
  fs.writeFileSync(path.join(replaced, "marker"), "previous");
  fs.mkdirSync(temporary, { recursive: true });
  fs.writeFileSync(path.join(root, "current.json"), JSON.stringify({ path: directory }));
  setupInternals.recoverInterruptedRuntime({
    root,
    runtimeId: "runtime-a",
    directory,
    pointer: path.join(root, "current.json"),
  });
  assert.equal(fs.readFileSync(path.join(directory, "marker"), "utf8"), "previous");
  assert.equal(fs.existsSync(temporary), false);
});

test("relocates Linux venv symlinks after the runtime directory is renamed", {
  skip: process.platform === "win32",
}, (t) => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "doudou-venv-relocate-"));
  t.after(() => fs.rmSync(root, { recursive: true, force: true }));
  const previous = path.join(root, ".runtime.install-fixture");
  const directory = path.join(root, "runtime");
  const bootstrapPython = path.join(previous, "python", "bin", "python3");
  const scripts = path.join(previous, "venv", "bin");
  fs.mkdirSync(path.dirname(bootstrapPython), { recursive: true });
  fs.mkdirSync(scripts, { recursive: true });
  fs.writeFileSync(bootstrapPython, "fixture");
  fs.writeFileSync(path.join(previous, "venv", "pyvenv.cfg"), `home = ${previous}/python/bin\n`);
  fs.writeFileSync(path.join(scripts, "activate"), `VIRTUAL_ENV=${previous}/venv\n`);
  fs.symlinkSync(bootstrapPython, path.join(scripts, "python"));

  fs.renameSync(previous, directory);
  const relocatedPython = path.join(directory, "venv", "bin", "python");
  assert.equal(fs.existsSync(relocatedPython), false);

  setupInternals.relocateVenv(directory, previous);

  assert.equal(fs.realpathSync(relocatedPython), path.join(directory, "python", "bin", "python3"));
  assert.equal(fs.readFileSync(path.join(directory, "venv", "pyvenv.cfg"), "utf8").includes(previous), false);
  assert.equal(fs.readFileSync(path.join(directory, "venv", "bin", "activate"), "utf8").includes(previous), false);
});

test("setup cancellation terminates its managed process tree", async () => {
  const child = spawn(process.execPath, ["-e", "setInterval(() => {}, 1000)"], {
    detached: process.platform !== "win32",
    stdio: "ignore",
    windowsHide: true,
  });
  await new Promise((resolve, reject) => {
    child.once("spawn", resolve);
    child.once("error", reject);
  });
  await setupInternals.terminateTree(child);
  await new Promise((resolve) => {
    if (child.exitCode !== null) return resolve();
    child.once("close", resolve);
    setTimeout(resolve, 3000);
  });
  assert.equal(setupInternals.processAlive(child.pid), false);
});
