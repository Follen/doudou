import { spawnSync } from "node:child_process";
import fs from "node:fs";
import net from "node:net";
import os from "node:os";
import path from "node:path";

import { clearManagedInstall, readManagedInstall } from "./managed-install.mjs";
import { resolveSkillDirectory } from "./options.mjs";
import { doudouCacheRoot } from "./python.mjs";
import { INSTALL_STAMP } from "./skill-installer.mjs";

const PACKAGE_NAME = "@follenfang/doudou";
const INSTALL_STAMP_SCHEMA = "doudou.npm-skill-install.v1";
const BROKER_REGISTRATION_SCHEMA = "doudou.broker-registration.v1";

export class ManagedUninstallError extends Error {
  constructor(message, details) {
    super(message);
    this.name = "ManagedUninstallError";
    this.code = "DOUDOU_UNINSTALL_FAILED";
    this.details = details;
  }
}

function isWithin(candidate, root) {
  const relative = path.relative(path.resolve(root), path.resolve(candidate));
  return relative === "" || (!relative.startsWith("..") && !path.isAbsolute(relative));
}

function sleep(milliseconds) {
  return new Promise((resolve) => setTimeout(resolve, milliseconds));
}

function processAlive(pid) {
  try {
    process.kill(pid, 0);
    return true;
  } catch {
    return false;
  }
}

function readJson(filename) {
  return JSON.parse(fs.readFileSync(filename, "utf8"));
}

async function sendBrokerShutdown(registration, timeoutMs) {
  const descriptor = readJson(registration.descriptor);
  if (
    descriptor?.schema !== "doudou.broker-descriptor.v1" ||
    Number(descriptor.pid) !== Number(registration.pid)
  ) {
    throw new Error("Broker descriptor does not match its managed registration.");
  }
  const token = fs.readFileSync(registration.token, "utf8").trim();
  const endpoint = descriptor.endpoint || {};
  if (endpoint.transport !== "tcp" || endpoint.host !== "127.0.0.1" || !endpoint.port) {
    throw new Error("Broker registration has an unsupported endpoint.");
  }
  await new Promise((resolve, reject) => {
    const socket = net.createConnection({ host: endpoint.host, port: Number(endpoint.port) });
    const timer = setTimeout(() => {
      socket.destroy();
      reject(new Error("Broker shutdown request timed out."));
    }, timeoutMs);
    let response = "";
    socket.setEncoding("utf8");
    socket.on("connect", () => {
      socket.write(
        `${JSON.stringify({
          token,
          request_id: `uninstall-${process.pid}-${Date.now()}`,
          command: "shutdown",
          semantics: "control",
          params: {},
        })}\n`,
      );
    });
    socket.on("data", (chunk) => {
      response += chunk;
      if (!response.includes("\n")) {
        return;
      }
      clearTimeout(timer);
      socket.end();
      try {
        const parsed = JSON.parse(response.split("\n", 1)[0]);
        if (!parsed.ok) {
          reject(new Error("Broker rejected the shutdown request."));
          return;
        }
        resolve();
      } catch (error) {
        reject(new Error("Broker returned an invalid shutdown response.", { cause: error }));
      }
    });
    socket.on("error", (error) => {
      clearTimeout(timer);
      reject(error);
    });
  });
}

export function discoverManagedBrokerProcesses(
  runtimeRoot,
  { platform = process.platform, run = spawnSync } = {},
) {
  const root = path.resolve(runtimeRoot);
  if (!fs.existsSync(root)) {
    return [];
  }
  if (platform === "win32") {
    const script =
      "Get-CimInstance Win32_Process | Select-Object ProcessId,ExecutablePath,CommandLine | ConvertTo-Json -Compress";
    const result = run(
      "powershell",
      ["-NoProfile", "-NonInteractive", "-Command", script],
      { encoding: "utf8", windowsHide: true },
    );
    if (result.status !== 0) {
      throw new Error("Windows process inventory failed.");
    }
    const parsed = result.stdout.trim() ? JSON.parse(result.stdout) : [];
    const records = Array.isArray(parsed) ? parsed : [parsed];
    return records
      .filter((record) => {
        const executable = String(record.ExecutablePath || "");
        const command = String(record.CommandLine || "");
        return (
          executable &&
          isWithin(executable, root) &&
          /(?:^|\s)doudou\.features\.broker(?:\s|$)/.test(command)
        );
      })
      .map((record) => ({ pid: Number(record.ProcessId), executable: record.ExecutablePath }));
  }
  if (platform !== "linux" || !fs.existsSync("/proc")) {
    return [];
  }
  const records = [];
  for (const name of fs.readdirSync("/proc")) {
    if (!/^\d+$/.test(name)) {
      continue;
    }
    try {
      const executable = fs.realpathSync(path.join("/proc", name, "exe"));
      const command = fs
        .readFileSync(path.join("/proc", name, "cmdline"))
        .toString("utf8")
        .split("\0");
      if (
        isWithin(executable, root) &&
        command.some((value) => value === "doudou.features.broker")
      ) {
        records.push({ pid: Number(name), executable });
      }
    } catch {
      // Processes can exit while /proc is being inspected.
    }
  }
  return records;
}

async function waitForExit(pid, timeoutMs) {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    if (!processAlive(pid)) {
      return true;
    }
    await sleep(50);
  }
  return !processAlive(pid);
}

export async function stopManagedBrokers(
  runtimeRoot,
  {
    cacheRoot,
    platform = process.platform,
    timeoutMs = 10_000,
    discover = discoverManagedBrokerProcesses,
    terminate = process.kill,
  } = {},
) {
  const root = path.resolve(runtimeRoot);
  const registrationsRoot = path.join(cacheRoot || path.dirname(root), "brokers");
  const results = [];
  if (fs.existsSync(registrationsRoot)) {
    for (const filename of fs.readdirSync(registrationsRoot).filter((name) => name.endsWith(".json"))) {
      const absolute = path.join(registrationsRoot, filename);
      let registration;
      try {
        registration = readJson(absolute);
        if (
          registration?.schema !== BROKER_REGISTRATION_SCHEMA ||
          !isWithin(registration.runtime, root) ||
          !isWithin(registration.executable, registration.runtime)
        ) {
          throw new Error("Broker registration is not owned by the managed runtime.");
        }
        if (!processAlive(Number(registration.pid))) {
          fs.rmSync(absolute, { force: true });
          results.push({ pid: Number(registration.pid), status: "missing" });
          continue;
        }
        await sendBrokerShutdown(registration, timeoutMs);
        if (!(await waitForExit(Number(registration.pid), timeoutMs))) {
          throw new Error("Broker did not exit after accepting shutdown.");
        }
        fs.rmSync(absolute, { force: true });
        results.push({ pid: Number(registration.pid), status: "stopped" });
      } catch (error) {
        throw new ManagedUninstallError("A managed broker could not be stopped.", {
          object: absolute,
          reason: error.message,
        });
      }
    }
  }

  for (const processRecord of discover(root, { platform })) {
    if (!processAlive(processRecord.pid)) {
      continue;
    }
    try {
      if (platform === "win32") {
        const result = spawnSync(
          "taskkill",
          ["/PID", String(processRecord.pid), "/T", "/F"],
          { encoding: "utf8", windowsHide: true },
        );
        if (result.status !== 0 && processAlive(processRecord.pid)) {
          throw new Error("Windows could not terminate the managed broker tree.");
        }
      } else {
        terminate(processRecord.pid, "SIGTERM");
      }
      if (!(await waitForExit(processRecord.pid, timeoutMs))) {
        throw new Error("Managed broker did not exit.");
      }
      results.push({ pid: processRecord.pid, status: "stopped" });
    } catch (error) {
      throw new ManagedUninstallError("A managed broker could not be stopped.", {
        pid: processRecord.pid,
        reason: error.message,
      });
    }
  }
  return results;
}

function inspectManagedSkill(target) {
  const resolved = path.resolve(target);
  if (resolved === path.parse(resolved).root) {
    return { target: resolved, status: "preserved_unmanaged", reason: "filesystem_root" };
  }
  if (!fs.existsSync(resolved)) {
    return { target: resolved, status: "missing" };
  }
  try {
    const stamp = readJson(path.join(resolved, INSTALL_STAMP));
    if (
      stamp?.schema !== INSTALL_STAMP_SCHEMA ||
      stamp?.package_name !== PACKAGE_NAME ||
      (stamp.target && path.resolve(stamp.target) !== resolved)
    ) {
      return { target: resolved, status: "preserved_unmanaged", reason: "ownership_mismatch" };
    }
  } catch {
    return { target: resolved, status: "preserved_unmanaged", reason: "ownership_unknown" };
  }
  return { target: resolved, status: "managed" };
}

export async function cleanupManagedInstall({
  platform = process.platform,
  env = process.env,
  home = os.homedir(),
  extraTargets = [],
  stopBrokers = stopManagedBrokers,
  removePath = (target) => fs.rmSync(target, { recursive: true, force: true }),
} = {}) {
  const cacheRoot = doudouCacheRoot({ platform, env, home });
  const runtimes = path.join(cacheRoot, "runtimes");
  const registry = readManagedInstall({ platform, env, home });
  const defaultTarget = resolveSkillDirectory({ env, home });
  const targets = [
    ...new Set([
      ...registry.targets.map((entry) => path.resolve(entry.path)),
      path.resolve(defaultTarget),
      ...extraTargets.map((target) => path.resolve(target)),
    ]),
  ];
  const result = {
    schema: "doudou.npm-uninstall-result.v1",
    brokers: [],
    skills: [],
    runtime: { path: runtimes, status: "missing" },
    browser_cache: "preserved",
    project_data: "preserved",
  };

  result.brokers = await stopBrokers(runtimes, { cacheRoot, platform });
  if (fs.existsSync(runtimes)) {
    try {
      removePath(runtimes);
      if (fs.existsSync(runtimes)) {
        throw new Error("path still exists after deletion");
      }
      result.runtime.status = "removed";
    } catch (error) {
      result.runtime.status = "failed";
      throw new ManagedUninstallError("The managed runtime could not be removed.", {
        result,
        object: runtimes,
        reason: error.message,
      });
    }
  }

  for (const target of targets) {
    const inspection = inspectManagedSkill(target);
    if (inspection.status !== "managed") {
      result.skills.push(inspection);
      continue;
    }
    try {
      removePath(inspection.target);
      if (fs.existsSync(inspection.target)) {
        throw new Error("path still exists after deletion");
      }
      result.skills.push({ target: inspection.target, status: "removed" });
    } catch (error) {
      result.skills.push({ target: inspection.target, status: "failed" });
      throw new ManagedUninstallError("A managed Skill could not be removed.", {
        result,
        object: inspection.target,
        reason: error.message,
      });
    }
  }
  clearManagedInstall({ platform, env, home });
  return result;
}
