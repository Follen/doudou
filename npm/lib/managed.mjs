import { spawnSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";

import { INSTALL_STAMP, INSTALL_STAMP_SCHEMA } from "./skill-installer.mjs";

const PACKAGE_NAME = "@follenfang/doudou";
const MANAGED_INSTALL_SCHEMA = "doudou.npm-managed-install.v1";
const MANAGED_INSTALL_FILE = "managed-install.json";

function atomicWriteJson(filename, value) {
  fs.mkdirSync(path.dirname(filename), { recursive: true });
  const temp = `${filename}.${process.pid}.${Date.now()}.tmp`;
  fs.writeFileSync(temp, `${JSON.stringify(value, null, 2)}\n`, { encoding: "utf8", mode: 0o600 });
  fs.renameSync(temp, filename);
}

export function readManagedInstall(doudouHome) {
  const filename = path.join(doudouHome, MANAGED_INSTALL_FILE);
  try {
    const value = JSON.parse(fs.readFileSync(filename, "utf8"));
    if (value?.schema === MANAGED_INSTALL_SCHEMA && Array.isArray(value.targets)) {
      return value;
    }
  } catch {
    // Missing or unreadable registry falls back to empty.
  }
  return { schema: MANAGED_INSTALL_SCHEMA, package_name: PACKAGE_NAME, targets: [] };
}

export function recordManagedSkill(target, doudouHome) {
  const resolved = path.resolve(target);
  if (resolved === path.parse(resolved).root) return;
  const state = readManagedInstall(doudouHome);
  const targets = state.targets
    .filter((entry) => typeof entry?.path === "string" && path.resolve(entry.path) !== resolved)
    .map((entry) => ({ path: path.resolve(entry.path), registered_at: entry.registered_at }));
  targets.push({ path: resolved, registered_at: new Date().toISOString() });
  targets.sort((a, b) => a.path.localeCompare(b.path));
  atomicWriteJson(path.join(doudouHome, MANAGED_INSTALL_FILE), {
    schema: MANAGED_INSTALL_SCHEMA,
    package_name: PACKAGE_NAME,
    targets,
  });
}

function npmCommand(platform = process.platform) {
  return platform === "win32" ? "npm.cmd" : "npm";
}

function isNpxInstall(packageRoot) {
  return path.resolve(packageRoot).split(path.sep).includes("_npx");
}

function compareSemver(a, b) {
  const pa = a.split("-")[0].split(".").map(Number);
  const pb = b.split("-")[0].split(".").map(Number);
  for (let i = 0; i < 3; i += 1) {
    if ((pa[i] || 0) !== (pb[i] || 0)) return (pa[i] || 0) - (pb[i] || 0);
  }
  return 0;
}

export function runUpdate({ currentVersion, check, packageRoot, platform = process.platform }) {
  const view = spawnSync(npmCommand(platform), ["view", PACKAGE_NAME, "version"], {
    encoding: "utf8",
    windowsHide: true,
    shell: platform === "win32",
  });
  if (view.status !== 0) {
    throw new Error(`Failed querying npm registry: ${view.stderr || view.stdout || "npm view failed"}`);
  }
  const latest = view.stdout.trim();
  const updateAvailable = compareSemver(latest, currentVersion) > 0;
  const npxMode = isNpxInstall(packageRoot);

  if (check || !updateAvailable || npxMode) {
    return {
      schema: "doudou.npm-update-result.v1",
      current_version: currentVersion,
      latest_version: latest,
      update_available: updateAvailable,
      mode: npxMode ? "npx" : check ? "check" : "current",
      updated: false,
    };
  }

  const install = spawnSync(npmCommand(platform), ["install", "-g", `${PACKAGE_NAME}@${latest}`], {
    encoding: "utf8",
    windowsHide: true,
    shell: platform === "win32",
  });
  if (install.status !== 0) {
    throw new Error(`Global update failed: ${install.stderr || install.stdout}`);
  }
  return {
    schema: "doudou.npm-update-result.v1",
    current_version: currentVersion,
    latest_version: latest,
    update_available: true,
    mode: "global",
    updated: true,
  };
}

function inspectManagedSkill(target) {
  const resolved = path.resolve(target);
  if (resolved === path.parse(resolved).root) {
    return { target: resolved, status: "preserved_unmanaged" };
  }
  if (!fs.existsSync(resolved)) {
    return { target: resolved, status: "missing" };
  }
  try {
    const stamp = JSON.parse(fs.readFileSync(path.join(resolved, INSTALL_STAMP), "utf8"));
    if (stamp?.schema === INSTALL_STAMP_SCHEMA && stamp?.package_name === PACKAGE_NAME) {
      return { target: resolved, status: "managed" };
    }
  } catch {
    // Not a managed directory.
  }
  return { target: resolved, status: "preserved_unmanaged" };
}

export function runUninstall({
  doudouHome,
  target,
  packageRoot,
  cachedBin,
  platform = process.platform,
}) {
  let brokersStopped = false;
  if (cachedBin && fs.existsSync(cachedBin)) {
    const stop = spawnSync(cachedBin, ["--home", doudouHome, "broker", "stop-all"], {
      encoding: "utf8",
      windowsHide: true,
    });
    brokersStopped = stop.status === 0;
  }

  const state = readManagedInstall(doudouHome);
  const targets = [
    ...new Set([...state.targets.map((entry) => path.resolve(entry.path)), path.resolve(target)]),
  ];
  const skills = [];
  for (const item of targets) {
    const inspection = inspectManagedSkill(item);
    if (inspection.status === "managed") {
      fs.rmSync(inspection.target, { recursive: true, force: true });
      skills.push({ target: inspection.target, status: "removed" });
    } else {
      skills.push(inspection);
    }
  }

  const binDir = path.join(doudouHome, "bin");
  let binariesRemoved = false;
  if (fs.existsSync(binDir)) {
    fs.rmSync(binDir, { recursive: true, force: true });
    binariesRemoved = true;
  }
  fs.rmSync(path.join(doudouHome, MANAGED_INSTALL_FILE), { force: true });

  let npmRemoved = false;
  if (!isNpxInstall(packageRoot)) {
    const uninstall = spawnSync(npmCommand(platform), ["uninstall", "-g", PACKAGE_NAME], {
      encoding: "utf8",
      windowsHide: true,
      shell: platform === "win32",
    });
    npmRemoved = uninstall.status === 0;
  }

  return {
    schema: "doudou.npm-uninstall-result.v1",
    brokers_stopped: brokersStopped,
    skills,
    binaries_removed: binariesRemoved,
    npm_package_removed: npmRemoved,
    profiles_and_messages: "preserved",
    doudou_home: doudouHome,
  };
}
