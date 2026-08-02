import { spawnSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

const packageRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..", "..");
const temporary = fs.mkdtempSync(path.join(os.tmpdir(), "doudou-npm-install-"));
const npmCli = process.env.npm_execpath;
if (!npmCli) {
  throw new Error("npm_execpath is required. Run this check through npm run test:install.");
}

function run(command, args, options = {}) {
  const result = spawnSync(command, args, {
    cwd: options.cwd || packageRoot,
    encoding: "utf8",
    env: options.env || process.env,
    shell: options.shell || false,
    windowsHide: true,
  });
  if (result.status !== 0) {
    throw new Error(
      `${command} ${args.join(" ")} failed (${result.status}):\n${result.stdout}\n${result.stderr}`,
    );
  }
  return result;
}

function installedCommand(prefix) {
  return process.platform === "win32"
    ? path.join(prefix, "doudou.cmd")
    : path.join(prefix, "bin", "doudou");
}

function runNpm(args, options = {}) {
  return run(process.execPath, [npmCli, ...args], options);
}

function runInstalled(prefix, args) {
  const command = installedCommand(prefix);
  if (process.platform !== "win32") {
    return run(command, args);
  }
  return run(process.env.ComSpec || "cmd.exe", ["/d", "/c", command, ...args]);
}

try {
  const packDirectory = path.join(temporary, "pack");
  fs.mkdirSync(packDirectory);
  const pack = runNpm([
    "pack",
    "--json",
    "--ignore-scripts",
    "--pack-destination",
    packDirectory,
  ]);
  const tarballName = JSON.parse(pack.stdout)[0].filename;
  const tarball = path.join(packDirectory, tarballName);

  const normalPrefix = path.join(temporary, "normal-prefix");
  const normalHome = path.join(temporary, "normal-agent");
  runNpm(["install", "-g", "--prefix", normalPrefix, tarball], {
    env: { ...process.env, DOUDOU_AGENT_HOME: normalHome },
  });
  const normalSkill = path.join(normalHome, "skills", "doudou");
  if (!fs.existsSync(path.join(normalSkill, ".doudou-npm-install.json"))) {
    throw new Error("postinstall did not install the Skill.");
  }
  const normalVersion = runInstalled(normalPrefix, [
    "--skill-dir",
    normalSkill,
    "--version",
  ]);
  if (normalVersion.stdout.trim() !== "2.1.1") {
    throw new Error(`Unexpected global CLI version: ${normalVersion.stdout}`);
  }

  const ignoredPrefix = path.join(temporary, "ignored-prefix");
  const ignoredSkill = path.join(temporary, "ignored-skill");
  runNpm([
    "install",
    "-g",
    "--ignore-scripts",
    "--prefix",
    ignoredPrefix,
    tarball,
  ]);
  const ignoredVersion = runInstalled(ignoredPrefix, [
    "--skill-dir",
    ignoredSkill,
    "--version",
  ]);
  if (
    ignoredVersion.stdout.trim() !== "2.1.1" ||
    !fs.existsSync(path.join(ignoredSkill, ".doudou-npm-install.json"))
  ) {
    throw new Error("The CLI did not recover from --ignore-scripts installation.");
  }

  const npxSkill = path.join(temporary, "npx-skill");
  const npxVersion = runNpm(
    [
      "exec",
      "--yes",
      `--package=${tarball}`,
      "--",
      "doudou",
      "--skill-dir",
      npxSkill,
      "--version",
    ],
  );
  if (
    npxVersion.stdout.trim() !== "2.1.1" ||
    !fs.existsSync(path.join(npxSkill, ".doudou-npm-install.json"))
  ) {
    throw new Error("npx did not install and run the Skill.");
  }
  process.stdout.write("Global, npx, and --ignore-scripts installs passed.\n");
} finally {
  fs.rmSync(temporary, { recursive: true, force: true });
}
