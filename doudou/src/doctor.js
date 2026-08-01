"use strict";

const childProcess = require("node:child_process");
const fs = require("node:fs");
const fsp = require("node:fs/promises");
const path = require("node:path");
const { ensureDir, now } = require("./core");
const { migrateLegacyProfileRoot, profileRoot } = require("./profile");

const REQUIRED_PLAYWRIGHT = "1.62.0";

function commandVersion(command, args) {
  const result = process.platform === "win32"
    ? childProcess.spawnSync(process.env.ComSpec || "cmd.exe", ["/d", "/s", "/c", [command, ...args].join(" ")], { encoding: "utf8", windowsHide: true })
    : childProcess.spawnSync(command, args, { encoding: "utf8" });
  return result.status === 0 ? result.stdout.trim() : null;
}

async function writable(directory) {
  try {
    await ensureDir(directory);
    const probe = path.join(directory, `.doctor-${process.pid}-${Date.now()}`);
    await fsp.writeFile(probe, "ok", "utf8");
    await fsp.rm(probe, { force: true });
    return true;
  } catch {
    return false;
  }
}

async function runDoctor({ projectRoot, launchProbe = true } = {}) {
  const issues = [];
  await migrateLegacyProfileRoot(projectRoot);
  const setupCommand = `node "${path.join(__dirname, "..", "bin", "doudou.js")}" setup`;
  const nodeVersion = process.versions.node;
  const nodeMajor = Number(nodeVersion.split(".")[0]);
  if (nodeMajor < 20) issues.push({ code: "NODE_VERSION_UNSUPPORTED", message: `Node.js >=20 is required; found ${nodeVersion}.`, repair_command: null });
  const npmVersion = commandVersion("npm", ["--version"]);
  if (!npmVersion) issues.push({ code: "NPM_MISSING", message: "npm was not found.", repair_command: null });

  let installedVersion = null;
  let chromiumPath = null;
  let chromiumVersion = null;
  let launchOk = false;
  try {
    installedVersion = require("playwright/package.json").version;
  } catch {
    issues.push({ code: "PLAYWRIGHT_MISSING", message: `Local Playwright ${REQUIRED_PLAYWRIGHT} is missing.`, repair_command: setupCommand });
  }
  if (installedVersion && installedVersion !== REQUIRED_PLAYWRIGHT) {
    issues.push({ code: "PLAYWRIGHT_VERSION_MISMATCH", message: `Expected Playwright ${REQUIRED_PLAYWRIGHT}; found ${installedVersion}.`, repair_command: setupCommand });
  }
  if (installedVersion === REQUIRED_PLAYWRIGHT) {
    const { chromium } = require("playwright");
    chromiumPath = chromium.executablePath();
    if (!fs.existsSync(chromiumPath)) {
      issues.push({ code: "CHROMIUM_MISSING", message: "The Chromium revision required by Playwright is missing.", repair_command: setupCommand });
    } else if (launchProbe) {
      try {
        const browser = await chromium.launch({ headless: true });
        chromiumVersion = browser.version();
        await browser.close();
        launchOk = true;
      } catch (error) {
        issues.push({ code: "CHROMIUM_LAUNCH_FAILED", message: String(error.message).split("\n")[0], repair_command: setupCommand });
      }
    } else {
      launchOk = true;
    }
  }

  const directories = {
    project: projectRoot,
    profiles: profileRoot(projectRoot),
    message: path.join(projectRoot, "message"),
  };
  const filesystem = {};
  for (const [name, directory] of Object.entries(directories)) {
    filesystem[name] = { path: directory, writable: await writable(directory) };
    if (!filesystem[name].writable) issues.push({ code: `FILESYSTEM_${name.toUpperCase()}_NOT_WRITABLE`, message: `Directory is not writable: ${directory}.`, repair_command: null });
  }
  const setupCodes = new Set(["PLAYWRIGHT_MISSING", "PLAYWRIGHT_VERSION_MISMATCH", "CHROMIUM_MISSING"]);
  const status = issues.length === 0 ? "ok" : issues.every((issue) => setupCodes.has(issue.code)) ? "setup_required" : "failed";
  return {
    schema: "doudou.doctor.v1",
    checked_at: now(),
    status,
    node: { version: nodeVersion, supported: nodeMajor >= 20 },
    npm: { version: npmVersion, available: Boolean(npmVersion) },
    playwright: { expected_version: REQUIRED_PLAYWRIGHT, installed_version: installedVersion, matches: installedVersion === REQUIRED_PLAYWRIGHT },
    chromium: { executable_path: chromiumPath, browser_version: chromiumVersion, managed_by: `playwright@${REQUIRED_PLAYWRIGHT}`, exists: Boolean(chromiumPath && fs.existsSync(chromiumPath)), launch_ok: launchOk, headless: true },
    filesystem,
    issues,
  };
}

async function runSetup({ skillRoot, onProgress = () => {} }) {
  const npm = "npm";
  const npx = "npx";
  const steps = [
    { name: "npm_ci", command: npm, args: ["ci"] },
    { name: "chromium_install", command: npx, args: ["playwright", "install", "chromium"] },
  ];
  for (const step of steps) {
    onProgress({ step: step.name, status: "started", command: [step.command, ...step.args].join(" ") });
    const result = process.platform === "win32"
      ? childProcess.spawnSync(process.env.ComSpec || "cmd.exe", ["/d", "/s", "/c", [step.command, ...step.args].join(" ")], { cwd: skillRoot, encoding: "utf8", stdio: ["ignore", "pipe", "pipe"], windowsHide: true })
      : childProcess.spawnSync(step.command, step.args, { cwd: skillRoot, encoding: "utf8", stdio: ["ignore", "pipe", "pipe"] });
    if (result.status !== 0) {
      const error = new Error(`Setup step failed: ${step.name}.`);
      error.code = "SETUP_STEP_FAILED";
      error.details = { step: step.name, status: result.status, retry_command: [step.command, ...step.args].join(" "), stderr: result.stderr.trim().slice(-2000) };
      throw error;
    }
    onProgress({ step: step.name, status: "completed" });
  }
}

module.exports = { REQUIRED_PLAYWRIGHT, runDoctor, runSetup };
