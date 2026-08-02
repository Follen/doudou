import crypto from "node:crypto";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { spawn, spawnSync } from "node:child_process";

import { controlledDownload, DownloadError } from "./controlled-download.mjs";
import { doudouCacheRoot, legacyDoudouRoot } from "./python.mjs";

export class SetupError extends Error {
  constructor(code, message, details = {}, options = {}) {
    super(message, options);
    this.name = "SetupError";
    this.code = code;
    this.details = details;
  }
}

function atomicJson(filename, value) {
  fs.mkdirSync(path.dirname(filename), { recursive: true });
  const temporary = `${filename}.${process.pid}.tmp`;
  fs.writeFileSync(temporary, `${JSON.stringify(value, null, 2)}\n`, { mode: 0o600 });
  fs.renameSync(temporary, filename);
}

export function readRuntimeManifest(skillDirectory) {
  const value = JSON.parse(fs.readFileSync(path.join(skillDirectory, "scripts", "runtime-manifest.json"), "utf8"));
  if (value.schema !== "doudou.runtime-manifest.v2") throw new SetupError("RUNTIME_MANIFEST_INVALID", "The Skill runtime manifest is unsupported.");
  return value;
}

export function platformAssetKey(platform = process.platform, arch = process.arch) {
  const key = `${platform}-${arch}`;
  if (!new Set(["win32-x64", "linux-x64"]).has(key)) throw new SetupError("RUNTIME_PLATFORM_UNSUPPORTED", `Doudou does not publish a runtime for ${key}.`);
  return key;
}

export function expectedManagedRuntime({ skillDirectory, platform = process.platform, arch = process.arch, env = process.env, home = os.homedir() }) {
  const manifest = readRuntimeManifest(skillDirectory);
  const key = platformAssetKey(platform, arch);
  const digest = crypto.createHash("sha256").update(JSON.stringify({ contract: manifest.runtime_contract, python: manifest.python, package: manifest.camoufox_package, browser: manifest.camoufox_browser, asset: manifest.camoufox_assets[key] })).digest("hex").slice(0, 12);
  const root = path.join(doudouCacheRoot({ platform, env, home }), "runtimes");
  const runtimeId = `py312-${manifest.runtime_contract}-${digest}`;
  return { manifest, key, root, runtimeId, directory: path.join(root, runtimeId), pointer: path.join(root, "current.json") };
}

export function managedPython(directory, platform = process.platform) {
  return path.join(directory, "venv", platform === "win32" ? "Scripts/python.exe" : "bin/python");
}

export function managedRuntimeReady(options) {
  const platform = options.platform || process.platform;
  const expected = expectedManagedRuntime({ ...options, platform });
  try {
    const pointer = JSON.parse(fs.readFileSync(expected.pointer, "utf8"));
    const record = JSON.parse(fs.readFileSync(path.join(pointer.path, "runtime.json"), "utf8"));
    const executable = managedPython(pointer.path, platform);
    if (pointer.runtime_id !== expected.runtimeId || record.status !== "verified" || !fs.existsSync(executable)) return false;
    if (options.probeRuntime === false) return true;
    const probe = spawnSync(executable, ["-c", "import sys; raise SystemExit(0 if sys.version_info[:2] == (3, 12) else 1)"], { windowsHide: true, stdio: "ignore", timeout: 5000 });
    return probe.status === 0;
  } catch {
    return false;
  }
}

function processAlive(pid) {
  try { process.kill(pid, 0); return true; } catch { return false; }
}

function emitProgress(event, options) {
  options.onProgress?.({ event: "setup-progress", at: new Date().toISOString(), ...event });
}

function writeOwner(paths, owner, patch, options) {
  if (patch.stage && patch.stage !== owner.stage) {
    for (const key of ["attempt", "error", "retrying", "speed_bps", "heartbeat", "completed"]) delete owner[key];
    if (!("downloaded" in patch)) delete owner.downloaded;
    if (!("total" in patch)) delete owner.total;
  }
  Object.assign(owner, patch, { updated_at: new Date().toISOString() });
  atomicJson(paths.state, owner);
  emitProgress(owner, options);
}

async function acquireSetupLock(cacheRoot, options) {
  fs.mkdirSync(cacheRoot, { recursive: true });
  const paths = { lock: path.join(cacheRoot, "setup.lock"), state: path.join(cacheRoot, "setup-state.json") };
  const token = crypto.randomUUID();
  for (;;) {
    try {
      const descriptor = fs.openSync(paths.lock, "wx", 0o600);
      const owner = { schema: "doudou.setup-state.v1", token, pid: process.pid, stage: "starting", started_at: new Date().toISOString(), updated_at: new Date().toISOString() };
      fs.writeFileSync(descriptor, `${JSON.stringify(owner)}\n`);
      fs.closeSync(descriptor);
      atomicJson(paths.state, owner);
      return { owned: true, paths, owner };
    } catch (error) {
      if (error.code !== "EEXIST") throw error;
      let current;
      try { current = JSON.parse(fs.readFileSync(paths.lock, "utf8")); } catch { current = null; }
      if (!current?.pid || !processAlive(current.pid)) {
        try { fs.rmSync(paths.lock); } catch {}
        continue;
      }
      let state = current;
      try { state = JSON.parse(fs.readFileSync(paths.state, "utf8")); } catch {}
      emitProgress({ stage: "waiting", owner_pid: current.pid, owner_stage: state.stage, downloaded: state.downloaded, total: state.total }, options);
      await new Promise((resolve) => setTimeout(resolve, options.lockPollMs ?? 1000));
    }
  }
}

function releaseSetupLock(lock) {
  if (!lock?.owned) return;
  try {
    const current = JSON.parse(fs.readFileSync(lock.paths.lock, "utf8"));
    if (current.token === lock.owner.token) fs.rmSync(lock.paths.lock, { force: true });
  } catch {}
}

async function terminateTree(child) {
  if (!child || child.exitCode !== null) return;
  if (process.platform === "win32") {
    await new Promise((resolve) => {
      const killer = spawn("taskkill", ["/pid", String(child.pid), "/T", "/F"], { windowsHide: true, stdio: "ignore" });
      killer.once("close", resolve);
      killer.once("error", resolve);
    });
  } else {
    try { process.kill(-child.pid, "SIGKILL"); } catch { child.kill("SIGKILL"); }
  }
}

async function runCommand(command, args, options = {}) {
  return await new Promise((resolve, reject) => {
    const child = spawn(command, args, {
      cwd: options.cwd,
      env: options.env || process.env,
      windowsHide: true,
      detached: process.platform !== "win32",
      stdio: ["ignore", "pipe", "pipe"],
    });
    let stderr = "";
    const forward = (stream, target) => stream.on("data", (chunk) => { target.write(chunk); if (target === process.stderr) stderr = `${stderr}${chunk}`.slice(-4000); });
    forward(child.stdout, process.stdout);
    forward(child.stderr, process.stderr);
    const abort = async () => { await terminateTree(child); };
    process.once("SIGINT", abort);
    process.once("SIGTERM", abort);
    child.once("error", reject);
    child.once("close", (code, signal) => {
      process.off("SIGINT", abort);
      process.off("SIGTERM", abort);
      if (code === 0) resolve({ code, signal });
      else reject(new SetupError(options.errorCode || "SETUP_PROCESS_FAILED", `${path.basename(command)} exited with status ${code ?? signal}.`, { command: path.basename(command), exit_code: code, stderr_tail: stderr }));
    });
  });
}

function pythonBootstrapExecutable(temporary, platform) {
  return path.join(temporary, "python", platform === "win32" ? "python.exe" : "bin/python3");
}

async function extractArchive(archive, destination, options = {}) {
  fs.mkdirSync(destination, { recursive: true });
  await runCommand("tar", ["-xf", archive, "-C", destination], { errorCode: options.errorCode || "RUNTIME_EXTRACT_FAILED" });
}

function browserCacheRoot(options) {
  return path.join(doudouCacheRoot(options), "browsers");
}

function validBrowserCache(cache, version, asset, platform) {
  const directory = path.join(cache, "browsers", "official", `${version}-${asset.sha256.slice(0, 8)}`);
  const executable = path.join(directory, platform === "win32" ? "camoufox.exe" : "camoufox-bin");
  try {
    const metadata = JSON.parse(fs.readFileSync(path.join(directory, "version.json"), "utf8"));
    return metadata.sha256 === asset.sha256 && Number(metadata.asset_size || asset.size) === asset.size && fs.existsSync(executable);
  } catch {
    return false;
  }
}

function scopedDownloadError(error, scope) {
  if (!(error instanceof DownloadError) || !error.code?.startsWith("DOWNLOAD_")) return error;
  return new SetupError(
    `${scope}_DOWNLOAD_${error.code.slice("DOWNLOAD_".length)}`,
    error.message,
    error.details,
    { cause: error },
  );
}

async function downloadRuntimeAsset(asset, options, scope) {
  try {
    return await controlledDownload(asset, options);
  } catch (error) {
    throw scopedDownloadError(error, scope);
  }
}

function currentPointer(expected) {
  try { return JSON.parse(fs.readFileSync(expected.pointer, "utf8")); } catch { return null; }
}

function cleanupLegacyInstall(options, currentRoot) {
  const legacy = legacyDoudouRoot(options);
  if (path.resolve(legacy) === path.resolve(currentRoot) || !fs.existsSync(legacy)) {
    return { status: "not_applicable", path: legacy };
  }
  const owned = fs.existsSync(path.join(legacy, "runtimes", "current.json"))
    || fs.existsSync(path.join(legacy, "setup-state.json"));
  if (!owned || fs.lstatSync(legacy).isSymbolicLink()) {
    return { status: "preserved_unrecognized", path: legacy };
  }
  fs.rmSync(legacy, { recursive: true, force: true });
  const result = { status: "removed", path: legacy, migrated_at: new Date().toISOString() };
  atomicJson(path.join(currentRoot, "migrations", "legacy-cache-v1.json"), {
    schema: "doudou.legacy-cache-migration.v1",
    ...result,
  });
  return result;
}

function recoverInterruptedRuntime(expected) {
  const pointer = currentPointer(expected);
  if (pointer?.path === expected.directory && !fs.existsSync(expected.directory)) {
    const prefix = `${path.basename(expected.directory)}.replaced-`;
    const candidates = fs.existsSync(expected.root)
      ? fs.readdirSync(expected.root)
        .filter((name) => name.startsWith(prefix))
        .map((name) => path.join(expected.root, name))
        .sort((left, right) => fs.statSync(right).mtimeMs - fs.statSync(left).mtimeMs)
      : [];
    if (candidates.length) fs.renameSync(candidates[0], expected.directory);
  }
  if (!fs.existsSync(expected.root)) return;
  const temporaryPrefix = `.${expected.runtimeId}.install-`;
  for (const name of fs.readdirSync(expected.root)) {
    if (name.startsWith(temporaryPrefix)) {
      fs.rmSync(path.join(expected.root, name), { recursive: true, force: true });
    }
  }
}

function relocateVenv(directory, previous) {
  const configuration = path.join(directory, "venv", "pyvenv.cfg");
  let text = fs.readFileSync(configuration, "utf8");
  text = text.split(previous).join(directory);
  fs.writeFileSync(configuration, text, "utf8");
  const scripts = path.join(directory, "venv", process.platform === "win32" ? "Scripts" : "bin");
  if (fs.existsSync(scripts)) {
    for (const name of fs.readdirSync(scripts)) {
      const filename = path.join(scripts, name);
      const entry = fs.lstatSync(filename);
      if (entry.isSymbolicLink()) {
        const target = fs.readlinkSync(filename);
        if (target.includes(previous)) {
          fs.unlinkSync(filename);
          fs.symlinkSync(target.split(previous).join(directory), filename);
        }
        continue;
      }
      if (!entry.isFile()) continue;
      try {
        const value = fs.readFileSync(filename, "utf8");
        if (value.includes(previous)) fs.writeFileSync(filename, value.split(previous).join(directory), "utf8");
      } catch {
        // Binary launchers are not used after setup and are left unchanged.
      }
    }
  }
}

export function setupDoctor(options) {
  const expected = expectedManagedRuntime(options);
  const cache = doudouCacheRoot(options);
  let state = null;
  try { state = JSON.parse(fs.readFileSync(path.join(cache, "setup-state.json"), "utf8")); } catch {}
  const partials = fs.existsSync(path.join(cache, "downloads")) ? fs.readdirSync(path.join(cache, "downloads")).filter((name) => name.endsWith(".part")).map((name) => ({ name, bytes: fs.statSync(path.join(cache, "downloads", name)).size })) : [];
  const ready = managedRuntimeReady(options);
  const ownerAlive = Boolean(state?.pid && !new Set(["complete", "failed"]).has(state.stage) && processAlive(state.pid));
  const browserAsset = expected.manifest.camoufox_assets[expected.key];
  const browserVersion = expected.manifest.camoufox_browser.split("/").at(-1);
  const browserCache = browserCacheRoot({ platform: options.platform || process.platform, env: options.env || process.env, home: options.home || os.homedir() });
  const cacheValid = validBrowserCache(browserCache, browserVersion, browserAsset, options.platform || process.platform);
  const updatedAt = Date.parse(state?.updated_at || "");
  const stalled = Boolean(
    state?.stage?.includes("download")
      && partials.some((item) => item.bytes > 0)
      && Number.isFinite(updatedAt)
      && Date.now() - updatedAt > expected.manifest.download.stall_timeout_seconds * 1000,
  ) || Boolean(state?.error_code?.endsWith("DOWNLOAD_STALLED"));
  return {
    schema: "doudou.setup-doctor.v1",
    status: ready ? "ok" : "setup_required",
    ready,
    expected_runtime_id: expected.runtimeId,
    current: currentPointer(expected),
    setup: state,
    owner_alive: ownerAlive,
    diagnosis: stalled ? "download_stalled" : state?.error_code || (ownerAlive ? "setup_in_progress" : null),
    partials,
    browser_cache: {
      version: browserVersion,
      sha256: browserAsset.sha256,
      valid: cacheValid,
      launch_verified: ready,
    },
    repair_command: ready ? null : "doudou setup",
  };
}

export async function ensureManagedRuntime(options) {
  const platform = options.platform || process.platform;
  const arch = options.arch || process.arch;
  const env = options.env || process.env;
  const home = options.home || os.homedir();
  const expected = expectedManagedRuntime({ ...options, platform, arch, env, home });
  const cache = doudouCacheRoot({ platform, env, home });
  if (!options.force && managedRuntimeReady({ ...options, platform, arch, env, home })) {
    return { status: "ready", reused: true, runtime: expected.directory, runtime_id: expected.runtimeId, legacy_migration: cleanupLegacyInstall({ platform, env, home }, cache) };
  }
  const lock = await acquireSetupLock(cache, options);
  let temporary;
  try {
    recoverInterruptedRuntime(expected);
    if (!options.force && managedRuntimeReady({ ...options, platform, arch, env, home })) return { status: "ready", reused: true, runtime: expected.directory, runtime_id: expected.runtimeId, waited: true };
    const owner = lock.owner;
    const downloads = path.join(cache, "downloads");
    fs.mkdirSync(downloads, { recursive: true });
    const downloadOptions = expected.manifest.download;
    const onProgress = (progress) => writeOwner(lock.paths, owner, progress, options);
    const common = { env, attempts: downloadOptions.attempts, backoffSeconds: downloadOptions.backoff_seconds, connectTimeoutMs: downloadOptions.connect_timeout_seconds * 1000, stallTimeoutMs: downloadOptions.stall_timeout_seconds * 1000, totalTimeoutMs: downloadOptions.total_timeout_seconds * 1000, heartbeatMs: downloadOptions.heartbeat_seconds * 1000, onProgress };
    const pythonAsset = expected.manifest.python.assets[expected.key];
    const pythonArchive = path.join(downloads, `python-${expected.manifest.python.version}-${expected.key}.tar.gz`);
    writeOwner(lock.paths, owner, { stage: "python-download" }, options);
    await downloadRuntimeAsset(pythonAsset, { ...common, destination: pythonArchive, identity: `python-${expected.manifest.python.version}-${expected.key}`, stage: "python-download" }, "PYTHON");
    temporary = path.join(expected.root, `.${expected.runtimeId}.install-${crypto.randomUUID()}`);
    fs.rmSync(temporary, { recursive: true, force: true });
    writeOwner(lock.paths, owner, { stage: "python-extract" }, options);
    await extractArchive(pythonArchive, temporary, { errorCode: "PYTHON_EXTRACT_FAILED" });
    const bootstrapPython = pythonBootstrapExecutable(temporary, platform);
    if (!fs.existsSync(bootstrapPython)) throw new SetupError("PYTHON_ASSET_INVALID", "The verified Python archive did not contain the expected executable.");
    writeOwner(lock.paths, owner, { stage: "venv-create" }, options);
    await runCommand(bootstrapPython, ["-m", "venv", path.join(temporary, "venv")], { errorCode: "VENV_CREATE_FAILED", env });
    const venvPython = managedPython(temporary, platform);
    writeOwner(lock.paths, owner, { stage: "dependencies" }, options);
    await runCommand(venvPython, ["-m", "pip", "install", "--disable-pip-version-check", "--require-hashes", "-r", path.join(options.skillDirectory, "scripts", "requirements.lock")], { errorCode: "DEPENDENCY_INSTALL_FAILED", env });
    await runCommand(venvPython, ["-m", "pip", "check"], { errorCode: "DEPENDENCY_VERIFY_FAILED", env });
    await runCommand(venvPython, ["-c", "from browserforge.fingerprints import Fingerprint; from camoufox.async_api import AsyncCamoufox; print('imports-ok')"], { errorCode: "DEPENDENCY_IMPORT_FAILED", env });
    const browserAsset = expected.manifest.camoufox_assets[expected.key];
    const browserArchive = path.join(downloads, `camoufox-${expected.manifest.camoufox_browser.split("/").at(-1)}-${expected.key}.zip`);
    const browserCache = browserCacheRoot({ platform, env, home });
    const browserVersion = expected.manifest.camoufox_browser.split("/").at(-1);
    if (validBrowserCache(browserCache, browserVersion, browserAsset, platform)) {
      writeOwner(lock.paths, owner, { stage: "browser-cache-hit", downloaded: browserAsset.size, total: browserAsset.size }, options);
    } else {
      writeOwner(lock.paths, owner, { stage: "browser-download" }, options);
      await downloadRuntimeAsset(browserAsset, { ...common, destination: browserArchive, identity: `camoufox-${expected.manifest.camoufox_browser}-${expected.key}`, stage: "browser-download" }, "BROWSER");
      writeOwner(lock.paths, owner, { stage: "browser-activate" }, options);
      const helperEnv = { ...env, PYTHONPATH: path.join(options.skillDirectory, "scripts") };
      await runCommand(venvPython, ["-m", "doudou.runtime.camoufox_asset", "install", "--archive", browserArchive, "--cache", browserCache, "--version", browserVersion, "--sha256", browserAsset.sha256, "--size", String(browserAsset.size)], { errorCode: "BROWSER_ACTIVATE_FAILED", env: helperEnv });
    }
    writeOwner(lock.paths, owner, { stage: "verification" }, options);
    const probeEnv = { ...env, PYTHONPATH: path.join(options.skillDirectory, "scripts") };
    await runCommand(venvPython, ["-m", "doudou.runtime.camoufox_asset", "probe", "--cache", browserCache, "--version", browserVersion], { errorCode: "RUNTIME_VERIFY_FAILED", env: probeEnv });
    atomicJson(path.join(temporary, "runtime.json"), { schema: "doudou.runtime-install.v2", status: "verified", runtime_id: expected.runtimeId, python_version: expected.manifest.python.version, camoufox_package: expected.manifest.camoufox_package, camoufox_browser: expected.manifest.camoufox_browser, browser_sha256: browserAsset.sha256, verified_at: new Date().toISOString() });
    writeOwner(lock.paths, owner, { stage: "switch" }, options);
    let replaced = null;
    if (fs.existsSync(expected.directory)) {
      replaced = `${expected.directory}.replaced-${Date.now()}`;
      fs.renameSync(expected.directory, replaced);
    }
    const previousLocation = temporary;
    try {
      fs.renameSync(temporary, expected.directory);
      temporary = null;
      relocateVenv(expected.directory, previousLocation);
      await runCommand(managedPython(expected.directory, platform), ["-c", "from browserforge.fingerprints import Fingerprint; from camoufox.async_api import AsyncCamoufox; print('relocation-ok')"], { errorCode: "RUNTIME_RELOCATION_FAILED", env });
    } catch (error) {
      fs.rmSync(expected.directory, { recursive: true, force: true });
      if (replaced) fs.renameSync(replaced, expected.directory);
      throw error;
    }
    atomicJson(expected.pointer, { schema: "doudou.runtime-pointer.v2", runtime_id: expected.runtimeId, path: expected.directory, updated_at: new Date().toISOString() });
    if (replaced) fs.rmSync(replaced, { recursive: true, force: true });
    writeOwner(lock.paths, owner, { stage: "complete", completed_at: new Date().toISOString() }, options);
    return { status: "installed", reused: false, runtime: expected.directory, runtime_id: expected.runtimeId, legacy_migration: cleanupLegacyInstall({ platform, env, home }, cache) };
  } catch (error) {
    if (temporary) fs.rmSync(temporary, { recursive: true, force: true });
    const code = error.code || (error instanceof DownloadError ? error.code : "RUNTIME_SETUP_FAILED");
    writeOwner(lock.paths, lock.owner, { stage: "failed", error_code: code }, options);
    throw error instanceof SetupError || error instanceof DownloadError ? error : new SetupError(code, error.message, {}, { cause: error });
  } finally {
    releaseSetupLock(lock);
  }
}

export const setupInternals = {
  acquireSetupLock,
  releaseSetupLock,
  processAlive,
  terminateTree,
  scopedDownloadError,
  writeOwner,
  validBrowserCache,
  browserCacheRoot,
  relocateVenv,
  recoverInterruptedRuntime,
  cleanupLegacyInstall,
};
