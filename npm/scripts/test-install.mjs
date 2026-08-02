import { spawnSync } from "node:child_process";
import crypto from "node:crypto";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

const packageRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..", "..");
const expectedVersion = JSON.parse(
  fs.readFileSync(path.join(packageRoot, "package.json"), "utf8"),
).version;
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

function runInstalled(prefix, args, options = {}) {
  const command = installedCommand(prefix);
  if (process.platform !== "win32") {
    return run(command, args, options);
  }
  return run(
    process.env.ComSpec || "cmd.exe",
    ["/d", "/c", command, ...args],
    options,
  );
}

function fileSha256(filename) {
  return crypto.createHash("sha256").update(fs.readFileSync(filename)).digest("hex");
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
  const cacheRoot = path.join(temporary, "user-cache");
  const doudouHome = path.join(cacheRoot, ".doudou");
  const managedEnvironment = {
    ...process.env,
    DOUDOU_AGENT_HOME: normalHome,
    DOUDOU_HOME: doudouHome,
  };
  runNpm(["install", "-g", "--prefix", normalPrefix, tarball], {
    env: managedEnvironment,
  });
  const normalSkill = path.join(normalHome, "skills", "doudou");
  if (!fs.existsSync(path.join(normalSkill, ".doudou-npm-install.json"))) {
    throw new Error("postinstall did not install the Skill.");
  }
  const normalVersion = runInstalled(
    normalPrefix,
    ["--skill-dir", normalSkill, "--version"],
    { env: managedEnvironment },
  );
  if (normalVersion.stdout.trim() !== expectedVersion) {
    throw new Error(`Unexpected global CLI version: ${normalVersion.stdout}`);
  }
  for (const alias of ["-v", "-version"]) {
    const aliasVersion = runInstalled(
      normalPrefix,
      ["--skill-dir", normalSkill, alias],
      { env: managedEnvironment },
    );
    if (aliasVersion.stdout.trim() !== expectedVersion) {
      throw new Error(`Unexpected ${alias} output: ${aliasVersion.stdout}`);
    }
  }
  const runtimeRoot = path.join(doudouHome, "runtimes");
  fs.mkdirSync(runtimeRoot, { recursive: true });
  fs.writeFileSync(path.join(runtimeRoot, "runtime.txt"), "managed runtime", "utf8");
  const project = path.join(temporary, "project");
  const profileRecord = path.join(project, "profile", "work", "profile.json");
  const messageRecord = path.join(
    project,
    "message",
    "profiles",
    "work",
    "messages.jsonl",
  );
  for (const [filename, content] of [
    [profileRecord, "profile data"],
    [messageRecord, "message data"],
  ]) {
    fs.mkdirSync(path.dirname(filename), { recursive: true });
    fs.writeFileSync(filename, content, "utf8");
  }
  const projectHashes = [fileSha256(profileRecord), fileSha256(messageRecord)];
  const normalUninstall = runInstalled(
    normalPrefix,
    ["--skill-dir", normalSkill, "uninstall"],
    { env: managedEnvironment },
  );
  const normalUninstallResult = JSON.parse(normalUninstall.stdout.trim());
  if (
    normalUninstallResult.npm.status !== "removed" ||
    fs.existsSync(normalSkill) ||
    fs.existsSync(runtimeRoot) ||
    fs.existsSync(installedCommand(normalPrefix)) ||
    fileSha256(profileRecord) !== projectHashes[0] ||
    fileSha256(messageRecord) !== projectHashes[1]
  ) {
    throw new Error(
      `Normal npm uninstall did not preserve the managed cleanup boundary: ${JSON.stringify({
        skill_exists: fs.existsSync(normalSkill),
        runtime_exists: fs.existsSync(runtimeRoot),
        launcher_exists: fs.existsSync(installedCommand(normalPrefix)),
        profile_matches: fileSha256(profileRecord) === projectHashes[0],
        message_matches: fileSha256(messageRecord) === projectHashes[1],
        stdout: normalUninstall.stdout.trim(),
        stderr: normalUninstall.stderr.trim(),
      })}`,
    );
  }

  const ignoredPrefix = path.join(temporary, "ignored-prefix");
  const ignoredSkill = path.join(temporary, "ignored-skill");
  runNpm(
    ["install", "-g", "--ignore-scripts", "--prefix", ignoredPrefix, tarball],
    { env: managedEnvironment },
  );
  const ignoredVersion = runInstalled(
    ignoredPrefix,
    ["--skill-dir", ignoredSkill, "--version"],
    { env: managedEnvironment },
  );
  if (
    ignoredVersion.stdout.trim() !== expectedVersion ||
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
    { env: managedEnvironment },
  );
  if (
    npxVersion.stdout.trim() !== expectedVersion ||
    !fs.existsSync(path.join(npxSkill, ".doudou-npm-install.json"))
  ) {
    throw new Error("npx did not install and run the Skill.");
  }
  fs.mkdirSync(runtimeRoot, { recursive: true });
  fs.writeFileSync(path.join(runtimeRoot, "runtime.txt"), "managed runtime", "utf8");
  const cleanup = runInstalled(
    ignoredPrefix,
    ["--skill-dir", ignoredSkill, "uninstall"],
    { env: managedEnvironment },
  );
  const cleanupResult = JSON.parse(cleanup.stdout.trim());
  if (
    cleanupResult.runtime.status !== "removed" ||
    fs.existsSync(ignoredSkill) ||
    fs.existsSync(npxSkill) ||
    fs.existsSync(runtimeRoot)
  ) {
    throw new Error("Explicit doudou uninstall did not remove managed Skills and runtime.");
  }
  if (
    fs.existsSync(installedCommand(ignoredPrefix)) ||
    fileSha256(profileRecord) !== projectHashes[0] ||
    fileSha256(messageRecord) !== projectHashes[1]
  ) {
    throw new Error("Self-uninstall changed project data or left the npm launcher.");
  }
  process.stdout.write(
    "Global, npx, ignore-scripts, and self-uninstall checks passed.\n",
  );
} finally {
  fs.rmSync(temporary, { recursive: true, force: true });
}
