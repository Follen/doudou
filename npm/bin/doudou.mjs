#!/usr/bin/env node

import { spawnSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

import { cachedBinaryPath, ensureBinary, resolvePlatformSpec } from "../lib/binary.mjs";
import { recordManagedSkill, runUninstall, runUpdate } from "../lib/managed.mjs";
import {
  firstCommand,
  parseLauncherOptions,
  resolveDoudouHome,
  resolveSkillDirectory,
} from "../lib/options.mjs";
import { ensureSkill } from "../lib/skill-installer.mjs";

const packageRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..", "..");
const packageVersion = JSON.parse(
  fs.readFileSync(path.join(packageRoot, "package.json"), "utf8"),
).version;

function emitError(code, message, details = undefined) {
  const record = { event: "error", code, message };
  if (details) record.details = details;
  process.stderr.write(`${JSON.stringify(record)}\n`);
}

async function main() {
  let options;
  try {
    options = parseLauncherOptions(process.argv.slice(2));
  } catch (error) {
    emitError("LAUNCHER_ARGUMENT_INVALID", error.message);
    return 2;
  }

  const target = resolveSkillDirectory({ explicit: options.skillDirectory });
  const doudouHome = resolveDoudouHome({ explicit: options.homeDirectory });
  const command = firstCommand(options.forwarded);

  if (options.forwarded.length === 1 && command === "uninstall") {
    try {
      let cachedBin;
      try {
        cachedBin = cachedBinaryPath(doudouHome, packageVersion, resolvePlatformSpec());
      } catch {
        cachedBin = undefined;
      }
      const result = runUninstall({ doudouHome, target, packageRoot, cachedBin });
      process.stdout.write(`${JSON.stringify(result)}\n`);
      return 0;
    } catch (error) {
      emitError(error.code || "DOUDOU_UNINSTALL_FAILED", error.message, error.details);
      return 1;
    }
  }

  try {
    await ensureSkill({ packageRoot, target });
    recordManagedSkill(target, doudouHome);
  } catch (error) {
    emitError("SKILL_INSTALL_FAILED", error.message, { target });
    return 1;
  }

  if (
    options.forwarded.length === 1 &&
    new Set(["--version", "-V", "-v"]).has(options.forwarded[0])
  ) {
    process.stdout.write(`doudou ${packageVersion}\n`);
    return 0;
  }

  if (command === "update") {
    try {
      const result = runUpdate({
        currentVersion: packageVersion,
        check: options.forwarded.includes("--check"),
        packageRoot,
      });
      process.stdout.write(`${JSON.stringify(result)}\n`);
      return 0;
    } catch (error) {
      emitError("UPDATE_FAILED", error.message);
      return 1;
    }
  }

  let binary;
  try {
    binary = await ensureBinary({ version: packageVersion, doudouHome });
  } catch (error) {
    emitError(error.code || "BINARY_INSTALL_FAILED", error.message, error.details);
    return 1;
  }

  const run = spawnSync(binary.path, options.forwarded, {
    stdio: "inherit",
    windowsHide: false,
  });
  if (run.error) {
    emitError("BINARY_LAUNCH_FAILED", run.error.message, { binary: binary.path });
    return 1;
  }
  return run.status ?? 1;
}

process.exitCode = await main();
