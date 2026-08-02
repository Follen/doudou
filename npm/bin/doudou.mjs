#!/usr/bin/env node

import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

import { detectCliCommand, parseLauncherOptions, resolveSkillDirectory } from "../lib/options.mjs";
import { ensureSkill } from "../lib/skill-installer.mjs";
import { recordManagedSkill } from "../lib/managed-install.mjs";
import { currentRuntime, runPython } from "../lib/python.mjs";
import { ensureManagedRuntime, managedRuntimeReady, setupDoctor } from "../lib/runtime-setup.mjs";
import { removeCurrentNpmPackage } from "../lib/self-remove.mjs";
import { cleanupManagedInstall } from "../lib/uninstall.mjs";
import { updateInstallation } from "../lib/update.mjs";

const packageRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..", "..");
const packageVersion = JSON.parse(fs.readFileSync(path.join(packageRoot, "package.json"), "utf8")).version;

function emitError(code, message, details = undefined) {
  const record = { event: "error", code, message };
  if (details) record.details = details;
  process.stderr.write(`${JSON.stringify(record)}\n`);
}

function setupOptions(target) {
  return {
    skillDirectory: target,
    onProgress(value) { process.stdout.write(`${JSON.stringify(value)}\n`); },
  };
}

async function main() {
  let options;
  try { options = parseLauncherOptions(process.argv.slice(2)); }
  catch (error) { emitError("LAUNCHER_ARGUMENT_INVALID", error.message); return 2; }
  const target = resolveSkillDirectory({ explicit: options.skillDirectory });
  if (options.forwarded.length === 1 && options.forwarded[0] === "uninstall") {
    try {
      const result = await cleanupManagedInstall({ extraTargets: [target] });
      const npm = removeCurrentNpmPackage({ packageRoot });
      process.stdout.write(`${JSON.stringify({ ...result, npm })}\n`);
      return 0;
    } catch (error) { emitError(error.code || "DOUDOU_UNINSTALL_FAILED", error.message, error.details); return 1; }
  }
  let ensured;
  try { ensured = await ensureSkill({ packageRoot, target }); recordManagedSkill(target); }
  catch (error) { emitError("SKILL_INSTALL_FAILED", error.message, { target }); return 1; }
  if (options.forwarded.length === 1 && new Set(["--version", "-version", "-v"]).has(options.forwarded[0])) {
    process.stdout.write(`${packageVersion}\n`);
    return 0;
  }
  const command = detectCliCommand(options.forwarded);
  if (command === "update") {
    try {
      const result = await updateInstallation({
        packageRoot,
        currentVersion: packageVersion,
        check: options.forwarded.includes("--check"),
        ensureSkill(updatedPackage) {
          const source = path.join(updatedPackage, "doudou");
          return ensureSkill({ packageRoot: updatedPackage, source, target });
        },
        ensureRuntime(updatedPackage) {
          return ensureManagedRuntime(setupOptions(path.join(updatedPackage, "doudou")));
        },
      });
      process.stdout.write(`${JSON.stringify(result)}\n`);
      return 0;
    } catch (error) { emitError("UPDATE_FAILED", error.message); return 1; }
  }
  if (command === "doctor" && !managedRuntimeReady(setupOptions(target))) {
    process.stdout.write(`${JSON.stringify({ event: "result", result: setupDoctor(setupOptions(target)) })}\n`);
    return 0;
  }
  if (command === "setup" || (command && !managedRuntimeReady(setupOptions(target)))) {
    try {
      const result = await ensureManagedRuntime(setupOptions(target));
      if (command === "setup") { process.stdout.write(`${JSON.stringify({ event: "result", result })}\n`); return 0; }
    } catch (error) { emitError(error.code || "RUNTIME_SETUP_FAILED", error.message, error.details); return 1; }
  }
  const runtime = currentRuntime();
  if (!runtime) { emitError("RUNTIME_SETUP_REQUIRED", "Run doudou setup to install the managed runtime."); return 1; }
  const script = path.join(target, "scripts", "doudou.py");
  const result = runPython(runtime, script, options.forwarded, { env: { ...process.env, DOUDOU_RUNTIME_ACTIVE: runtime.runtime_id } });
  if (result.error) { emitError("PYTHON_LAUNCH_FAILED", result.error.message); return 1; }
  return result.status ?? 1;
}

process.exitCode = await main();
