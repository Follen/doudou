"use strict";

const assert = require("node:assert/strict");
const fsp = require("node:fs/promises");
const os = require("node:os");
const path = require("node:path");
const test = require("node:test");
const { atomicWriteJson } = require("../src/core");
const { acquireProfileLock, createProfile, listProfiles, profilePaths, updateProfile } = require("../src/profile");

async function temporaryDirectory(t) {
  const directory = await fsp.mkdtemp(path.join(os.tmpdir(), "douyin-profile-"));
  t.after(() => fsp.rm(directory, { recursive: true, force: true }));
  return directory;
}

test("Profile metadata is isolated under lowercase profile and updates atomically", async (t) => {
  const root = await temporaryDirectory(t);
  const created = await createProfile(root, "工作/账号", "工作账号");
  assert.equal(created.metadata.profile_id, "工作~002F账号");
  assert.match(created.paths.browserData, /profile[\\/]工作~002F账号[\\/]browser-data$/);
  assert.match(created.paths.fingerprint, /profile[\\/]工作~002F账号[\\/]fingerprint\.json$/);
  const updated = await updateProfile(root, "工作/账号", { login_status: "valid", last_error_code: null });
  assert.equal(updated.login_status, "valid");
  const profiles = await listProfiles(root);
  assert.equal(profiles.length, 1);
  assert.equal(profiles[0].display_name, "工作账号");
});

test("legacy uppercase Profile root migrates to lowercase profile", async (t) => {
  const root = await temporaryDirectory(t);
  const legacy = path.join(root, "Profile", "work");
  await fsp.mkdir(path.join(legacy, "browser-data"), { recursive: true });
  await fsp.writeFile(path.join(legacy, "profile.json"), `${JSON.stringify({
    schema: "douyin-group-message.profile.v1",
    profile_id: "work",
    display_name: "work",
    browser_data_dir: "browser-data",
    created_at: "2026-07-31T00:00:00.000Z",
    updated_at: "2026-07-31T00:00:00.000Z",
    last_login_verified_at: null,
    login_status: "login_required",
    last_error_code: null,
  }, null, 2)}\n`, "utf8");

  const profiles = await listProfiles(root);
  assert.equal(profiles.length, 1);
  assert.equal(await fsp.stat(path.join(root, "profile", "work", "profile.json")).then((item) => item.isFile()), true);
  const entries = await fsp.readdir(root);
  assert.equal(entries.includes("Profile"), false);
});

test("a live Profile lock reports confirmation and releases cleanly", async (t) => {
  const root = await temporaryDirectory(t);
  await createProfile(root, "work");
  const first = await acquireProfileLock(root, "work");
  await assert.rejects(() => acquireProfileLock(root, "work"), (error) => {
    assert.equal(error.code, "PROFILE_IN_USE");
    assert.equal(error.details.requires_confirmation, true);
    assert.equal(error.details.owner.pid, process.pid);
    return true;
  });
  await first.release();
  const second = await acquireProfileLock(root, "work");
  await second.release();
});

test("a stale Profile lock is removed without asking to terminate", async (t) => {
  const root = await temporaryDirectory(t);
  await createProfile(root, "work");
  const paths = profilePaths(root, "work");
  await atomicWriteJson(paths.lock, {
    schema: "douyin-group-message.profile-lock.v1",
    profile_id: "work",
    pid: 2147483646,
    process_start_time: "2000-01-01T00:00:00.000Z",
    command: "douyin-group-message",
    lock_created_at: "2000-01-01T00:00:00.000Z",
  });
  const lock = await acquireProfileLock(root, "work");
  assert.equal(lock.owner.pid, process.pid);
  await lock.release();
});
