#!/usr/bin/env node

import path from "node:path";
import { fileURLToPath } from "node:url";

import {
  detectCliCommand,
  parseLauncherOptions,
  resolveSkillDirectory,
} from "../lib/options.mjs";
import { ensureSkill } from "../lib/skill-installer.mjs";
import { discoverPython, runPython, runtimeReady } from "../lib/python.mjs";

const packageRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..", "..");

function emitError(code, message, details = undefined) {
  const record = { event: "error", code, message };
  if (details) {
    record.details = details;
  }
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
  let ensured;
  try {
    ensured = await ensureSkill({ packageRoot, target });
  } catch (error) {
    emitError("SKILL_INSTALL_FAILED", error.message, { target });
    return 1;
  }
  if (options.forwarded.length === 1 && options.forwarded[0] === "--version") {
    process.stdout.write(`${ensured.distribution.packageRecord.version}\n`);
    return 0;
  }

  const python = discoverPython();
  if (!python) {
    emitError(
      "PYTHON_VERSION_UNSUPPORTED",
      "Doudou requires Python >=3.10,<4.0. Install Python and run the command again.",
    );
    return 1;
  }
  const script = path.join(target, "scripts", "doudou.py");
  const command = detectCliCommand(options.forwarded);
  if (
    command &&
    command !== "doctor" &&
    command !== "setup" &&
    !runtimeReady({ skillDirectory: target, python })
  ) {
    const setup = runPython(python, script, ["setup"]);
    if (setup.error) {
      emitError("RUNTIME_SETUP_FAILED", setup.error.message);
      return 1;
    }
    if (setup.status !== 0) {
      return setup.status ?? 1;
    }
  }
  const result = runPython(python, script, options.forwarded);
  if (result.error) {
    emitError("PYTHON_LAUNCH_FAILED", result.error.message);
    return 1;
  }
  return result.status ?? 1;
}

process.exitCode = await main();
