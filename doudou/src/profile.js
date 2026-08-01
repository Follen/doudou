"use strict";

const childProcess = require("node:child_process");
const fs = require("node:fs");
const fsp = require("node:fs/promises");
const path = require("node:path");
const {
  AppError,
  atomicWriteJson,
  ensureDir,
  now,
  readJson,
  safeSegment,
  sleep,
} = require("./core");
const { validateRecord } = require("./schemas");

function profileRoot(projectRoot) {
  return path.join(projectRoot, "profile");
}

function legacyProfileRoot(projectRoot) {
  return path.join(projectRoot, "Profile");
}

async function migrateLegacyProfileRoot(projectRoot) {
  const root = profileRoot(projectRoot);
  const legacy = legacyProfileRoot(projectRoot);
  const entries = await fsp.readdir(projectRoot, { withFileTypes: true }).catch((error) => {
    if (error.code === "ENOENT") return [];
    throw error;
  });
  const currentEntry = entries.find((entry) => entry.name === "profile");
  const legacyEntry = entries.find((entry) => entry.name === "Profile");
  if (!legacyEntry) return root;
  if (currentEntry) {
    throw new AppError("PROFILE_ROOT_CONFLICT", "Both profile/ and legacy Profile/ exist; merge them explicitly before continuing.", {
      profile_root: root,
      legacy_profile_root: legacy,
    });
  }

  const temporary = path.join(projectRoot, `.doudou-profile-migration-${process.pid}-${Date.now()}`);
  await fsp.rename(legacy, temporary);
  try {
    await fsp.rename(temporary, root);
  } catch (error) {
    await fsp.rename(temporary, legacy).catch(() => {});
    throw error;
  }
  return root;
}

function profilePaths(projectRoot, profileId) {
  const folderName = safeSegment(profileId);
  const directory = path.join(profileRoot(projectRoot), folderName);
  return {
    folderName,
    directory,
    browserData: path.join(directory, "browser-data"),
    fingerprint: path.join(directory, "fingerprint.json"),
    metadata: path.join(directory, "profile.json"),
    lock: path.join(directory, ".lock.json"),
  };
}

async function createProfile(projectRoot, profileId, displayName = profileId) {
  await migrateLegacyProfileRoot(projectRoot);
  const paths = profilePaths(projectRoot, profileId);
  if (fs.existsSync(paths.metadata)) {
    throw new AppError("PROFILE_EXISTS", `Profile already exists: ${profileId}.`, { profile_id: profileId });
  }
  await ensureDir(paths.browserData);
  const timestamp = now();
  const metadata = {
    schema: "doudou.profile.v1",
    profile_id: paths.folderName,
    display_name: displayName,
    browser_data_dir: "browser-data",
    created_at: timestamp,
    updated_at: timestamp,
    last_login_verified_at: null,
    login_status: "login_required",
    last_error_code: null,
  };
  validateRecord(metadata);
  await atomicWriteJson(paths.metadata, metadata);
  return { metadata, paths };
}

async function loadProfile(projectRoot, profileId) {
  await migrateLegacyProfileRoot(projectRoot);
  const paths = profilePaths(projectRoot, profileId);
  const metadata = await readJson(paths.metadata, { optional: true });
  if (!metadata) {
    throw new AppError("PROFILE_NOT_FOUND", `Profile not found: ${profileId}.`, { profile_id: profileId });
  }
  validateRecord(metadata, { path: paths.metadata });
  if (metadata.profile_id !== paths.folderName || metadata.browser_data_dir !== "browser-data") {
    throw new AppError("PROFILE_METADATA_MISMATCH", `Profile metadata does not match its directory: ${profileId}.`, { path: paths.metadata });
  }
  return { metadata, paths };
}

async function listProfiles(projectRoot) {
  const root = await migrateLegacyProfileRoot(projectRoot);
  await ensureDir(root);
  const entries = await fsp.readdir(root, { withFileTypes: true });
  const profiles = [];
  for (const entry of entries) {
    if (!entry.isDirectory()) continue;
    const filename = path.join(root, entry.name, "profile.json");
    const metadata = await readJson(filename, { optional: true });
    if (!metadata) continue;
    validateRecord(metadata, { path: filename });
    profiles.push(metadata);
  }
  profiles.sort((left, right) => left.created_at.localeCompare(right.created_at));
  return profiles;
}

async function updateProfile(projectRoot, profileId, patch) {
  const { metadata, paths } = await loadProfile(projectRoot, profileId);
  const next = { ...metadata, ...patch, updated_at: now() };
  validateRecord(next, { path: paths.metadata });
  await atomicWriteJson(paths.metadata, next);
  return next;
}

function powershellProcess(pid) {
  const script = [
    `$p = Get-CimInstance Win32_Process -Filter \"ProcessId = ${pid}\"`,
    "if ($null -eq $p) { exit 3 }",
    "$o = [ordered]@{ pid=[int]$p.ProcessId; created_at=$p.CreationDate.ToUniversalTime().ToString('o'); command_line=[string]$p.CommandLine; executable=[string]$p.ExecutablePath }",
    "$o | ConvertTo-Json -Compress",
  ].join("; ");
  const result = childProcess.spawnSync("powershell", ["-NoProfile", "-NonInteractive", "-Command", script], {
    encoding: "utf8",
    windowsHide: true,
  });
  if (result.status !== 0) return null;
  try {
    return JSON.parse(result.stdout.trim());
  } catch {
    return null;
  }
}

function posixProcess(pid) {
  const result = childProcess.spawnSync("ps", ["-p", String(pid), "-o", "lstart=", "-o", "args="], { encoding: "utf8" });
  if (result.status !== 0 || !result.stdout.trim()) return null;
  const output = result.stdout.trim();
  const createdText = output.slice(0, 24).trim();
  return {
    pid,
    created_at: new Date(createdText).toISOString(),
    command_line: output.slice(24).trim(),
    executable: null,
  };
}

function inspectProcess(pid) {
  if (!Number.isInteger(pid) || pid <= 0) return null;
  return process.platform === "win32" ? powershellProcess(pid) : posixProcess(pid);
}

function sameInstant(left, right) {
  const a = Date.parse(left);
  const b = Date.parse(right);
  return Number.isFinite(a) && Number.isFinite(b) && Math.abs(a - b) < 2000;
}

function redactedOwner(owner) {
  return {
    pid: owner.pid,
    process_start_time: owner.process_start_time,
    lock_created_at: owner.lock_created_at,
    command: owner.command,
    profile_id: owner.profile_id,
  };
}

async function terminateValidatedOwner(owner) {
  const actual = inspectProcess(owner.pid);
  const marker = "doudou";
  if (!actual || !sameInstant(actual.created_at, owner.process_start_time) || !String(actual.command_line).toLowerCase().includes(marker)) {
    throw new AppError("PROFILE_OWNER_IDENTITY_MISMATCH", "The lock owner no longer matches the recorded Skill process.", {
      owner: redactedOwner(owner),
    });
  }
  const result = process.platform === "win32"
    ? childProcess.spawnSync("taskkill", ["/PID", String(owner.pid), "/T", "/F"], { encoding: "utf8", windowsHide: true })
    : childProcess.spawnSync("kill", ["-TERM", String(owner.pid)], { encoding: "utf8" });
  if (result.status !== 0) {
    throw new AppError("PROFILE_OWNER_TERMINATION_FAILED", "Failed to terminate the confirmed Profile owner.", {
      owner: redactedOwner(owner),
      status: result.status,
    });
  }
}

async function acquireProfileLock(projectRoot, profileId, { terminateOwner = false } = {}) {
  const { paths, metadata } = await loadProfile(projectRoot, profileId);
  const actualSelf = inspectProcess(process.pid);
  const owner = {
    schema: "doudou.profile-lock.v1",
    profile_id: metadata.profile_id,
    pid: process.pid,
    process_start_time: actualSelf?.created_at || new Date(Date.now() - process.uptime() * 1000).toISOString(),
    command: "doudou",
    lock_created_at: now(),
  };

  for (let attempt = 0; attempt < 2; attempt += 1) {
    try {
      const handle = await fsp.open(paths.lock, "wx");
      await handle.writeFile(`${JSON.stringify(owner, null, 2)}\n`, "utf8");
      await handle.close();
      let released = false;
      return {
        owner,
        async release() {
          if (released) return;
          released = true;
          const current = await readJson(paths.lock, { optional: true });
          if (current?.pid === process.pid && current?.process_start_time === owner.process_start_time) {
            await fsp.rm(paths.lock, { force: true });
          }
        },
      };
    } catch (error) {
      if (error.code !== "EEXIST") throw error;
      const existing = await readJson(paths.lock, { optional: true });
      const actual = existing ? inspectProcess(existing.pid) : null;
      if (!existing || !actual || !sameInstant(actual.created_at, existing.process_start_time)) {
        await fsp.rm(paths.lock, { force: true });
        continue;
      }
      if (!terminateOwner) {
        throw new AppError("PROFILE_IN_USE", `Profile ${profileId} is already in use.`, {
          requires_confirmation: true,
          owner: redactedOwner(existing),
          retry_argument: "--terminate-owner",
        });
      }
      await terminateValidatedOwner(existing);
      for (let wait = 0; wait < 50 && fs.existsSync(paths.lock); wait += 1) {
        await sleep(100);
      }
      if (fs.existsSync(paths.lock)) {
        const post = inspectProcess(existing.pid);
        if (post) throw new AppError("PROFILE_LOCK_NOT_RELEASED", "The owner exited but the Profile lock was not released.");
        await fsp.rm(paths.lock, { force: true });
      }
    }
  }
  throw new AppError("PROFILE_LOCK_FAILED", `Could not acquire Profile lock: ${profileId}.`);
}

async function withProfileLock(projectRoot, profileId, options, callback) {
  const lock = await acquireProfileLock(projectRoot, profileId, options);
  try {
    return await callback();
  } finally {
    await lock.release();
  }
}

module.exports = {
  acquireProfileLock,
  createProfile,
  inspectProcess,
  listProfiles,
  loadProfile,
  migrateLegacyProfileRoot,
  profilePaths,
  profileRoot,
  updateProfile,
  withProfileLock,
};
