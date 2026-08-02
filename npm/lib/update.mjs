import fs from "node:fs";
import path from "node:path";
import { spawnSync } from "node:child_process";

const PACKAGE_NAME = "@follenfang/doudou";

function parseVersion(value) {
  const match = String(value).match(
    /^v?(\d+)\.(\d+)\.(\d+)(?:-([0-9A-Za-z.-]+))?(?:\+[0-9A-Za-z.-]+)?$/,
  );
  if (!match) throw new Error(`Invalid semantic version: ${value}`);
  return {
    core: match.slice(1, 4).map(Number),
    prerelease: match[4] ? match[4].split(".") : [],
  };
}

export function compareVersions(left, right) {
  const a = parseVersion(left);
  const b = parseVersion(right);
  for (let index = 0; index < 3; index += 1) {
    if (a.core[index] !== b.core[index]) return a.core[index] - b.core[index];
  }
  if (!a.prerelease.length || !b.prerelease.length) {
    return a.prerelease.length === b.prerelease.length
      ? 0
      : a.prerelease.length
        ? -1
        : 1;
  }
  const length = Math.max(a.prerelease.length, b.prerelease.length);
  for (let index = 0; index < length; index += 1) {
    const leftPart = a.prerelease[index];
    const rightPart = b.prerelease[index];
    if (leftPart === undefined || rightPart === undefined) {
      return leftPart === rightPart ? 0 : leftPart === undefined ? -1 : 1;
    }
    if (leftPart === rightPart) continue;
    const leftNumeric = /^\d+$/.test(leftPart);
    const rightNumeric = /^\d+$/.test(rightPart);
    if (leftNumeric && rightNumeric) return Number(leftPart) - Number(rightPart);
    if (leftNumeric !== rightNumeric) return leftNumeric ? -1 : 1;
    return leftPart < rightPart ? -1 : 1;
  }
  return 0;
}

function outputText(value) {
  return typeof value === "string" ? value.trim() : "";
}

export function resolveNpmCli(options = {}) {
  const env = options.env || process.env;
  const explicit = options.npmCli || env.npm_execpath;
  if (explicit) return explicit;
  const execPath = options.execPath || process.execPath;
  const adjacent = path.join(
    path.dirname(execPath),
    "node_modules",
    "npm",
    "bin",
    "npm-cli.js",
  );
  return fs.existsSync(adjacent) ? adjacent : null;
}

function npmInvocation(args, options = {}) {
  const env = options.env || process.env;
  const npmCli = resolveNpmCli(options);
  if (npmCli) {
    return spawnSync(options.execPath || process.execPath, [npmCli, ...args], {
      encoding: "utf8",
      windowsHide: true,
      env,
    });
  }
  if (process.platform === "win32") {
    return {
      status: null,
      stdout: "",
      stderr: "",
      error: new Error("npm-cli.js was not found next to the active Node.js runtime"),
    };
  }
  return spawnSync("npm", args, {
    encoding: "utf8",
    windowsHide: true,
    env,
  });
}

export function isNpxInvocation({ packageRoot, env = process.env } = {}) {
  const normalized = path.resolve(packageRoot).toLowerCase();
  return normalized.includes(`${path.sep}_npx${path.sep}`) || Boolean(env.npm_lifecycle_event === "npx");
}

export function latestVersion(options = {}) {
  const result = npmInvocation(["view", PACKAGE_NAME, "version", "--json"], options);
  if (result.status !== 0) {
    throw new Error(
      outputText(result.stderr)
        || result.error?.message
        || "npm registry version lookup failed",
    );
  }
  const parsed = JSON.parse(outputText(result.stdout));
  const latest = Array.isArray(parsed) ? parsed.at(-1) : parsed;
  if (typeof latest !== "string" || !latest.trim()) {
    throw new Error("npm registry returned an invalid package version");
  }
  return latest.trim();
}

export function globalPackageRoot(options = {}) {
  const result = npmInvocation(["root", "-g"], options);
  if (result.status !== 0) return null;
  const root = outputText(result.stdout);
  return root ? path.resolve(root, "@follenfang", "doudou") : null;
}

function runtimeContract(packageRoot) {
  try {
    return JSON.parse(
      fs.readFileSync(path.join(packageRoot, "doudou", "scripts", "runtime-manifest.json"), "utf8"),
    ).runtime_contract || null;
  } catch {
    return null;
  }
}

export async function updateInstallation({ packageRoot, currentVersion, check = false, env = process.env, npmCli, ensureSkill, ensureRuntime }) {
  const latest = latestVersion({ env, npmCli });
  const temporary = isNpxInvocation({ packageRoot, env });
  const globalRoot = globalPackageRoot({ env, npmCli });
  const global = !temporary && globalRoot && path.resolve(packageRoot) === globalRoot;
  const status = compareVersions(latest, currentVersion) > 0 ? "available" : "current";
  if (check || !global || status === "current") return { schema: "doudou.update.v1", mode: temporary ? "npx" : global ? "global" : "local", status, current: currentVersion, latest, changed: false };
  const previousContract = runtimeContract(packageRoot);
  const result = npmInvocation(["install", "-g", `${PACKAGE_NAME}@${latest}`], { env, npmCli });
  if (result.status !== 0) {
    npmInvocation(["install", "-g", `${PACKAGE_NAME}@${currentVersion}`], { env, npmCli });
    throw new Error(
      outputText(result.stderr)
        || result.error?.message
        || "npm global update failed",
    );
  }
  try {
    const updatedPackage = globalPackageRoot({ env, npmCli });
    if (!updatedPackage || !fs.existsSync(path.join(updatedPackage, "package.json"))) throw new Error("The upgraded global package was not found after npm completed.");
    const skill = await ensureSkill(updatedPackage);
    const nextContract = runtimeContract(updatedPackage);
    const runtime = previousContract !== nextContract && ensureRuntime
      ? await ensureRuntime(updatedPackage)
      : { status: "unchanged", contract: nextContract };
    return { schema: "doudou.update.v1", mode: "global", status: "updated", current: currentVersion, latest, changed: true, skill, runtime };
  } catch (error) {
    const rollback = npmInvocation(["install", "-g", `${PACKAGE_NAME}@${currentVersion}`], { env, npmCli });
    const restoredPackage = globalPackageRoot({ env, npmCli });
    if (rollback.status === 0 && restoredPackage && fs.existsSync(path.join(restoredPackage, "package.json"))) {
      try { await ensureSkill(restoredPackage); } catch {}
    }
    throw error;
  }
}
