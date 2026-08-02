import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { spawnSync } from "node:child_process";

const VERSION_PROBE =
  "import json,sys; print(json.dumps({'major':sys.version_info[0],'minor':sys.version_info[1],'micro':sys.version_info[2]}))";

export function pythonCandidates(platform = process.platform) {
  if (platform === "win32") {
    return [
      { command: "py", prefix: ["-3"] },
      { command: "python3", prefix: [] },
      { command: "python", prefix: [] },
    ];
  }
  return [
    { command: "python3", prefix: [] },
    { command: "python", prefix: [] },
  ];
}

export function supportedPythonVersion(version) {
  return version.major === 3 && version.minor >= 10;
}

export function discoverPython({ platform = process.platform, env = process.env } = {}) {
  for (const candidate of pythonCandidates(platform)) {
    const result = spawnSync(
      candidate.command,
      [...candidate.prefix, "-c", VERSION_PROBE],
      {
        encoding: "utf8",
        env,
        shell: false,
        windowsHide: true,
      },
    );
    if (result.status !== 0) {
      continue;
    }
    try {
      const version = JSON.parse(result.stdout.trim());
      if (supportedPythonVersion(version)) {
        return { ...candidate, version };
      }
    } catch {
      // Try the next stable candidate.
    }
  }
  return null;
}

function cacheRoot(platform, env, home) {
  if (platform === "win32") {
    return path.join(env.LOCALAPPDATA || path.join(home, "AppData", "Local"), "doudou");
  }
  return path.join(env.XDG_CACHE_HOME || path.join(home, ".cache"), "doudou");
}

export function expectedRuntime({ skillDirectory, python, platform = process.platform, env = process.env, home = os.homedir() }) {
  const manifest = JSON.parse(
    fs.readFileSync(path.join(skillDirectory, "scripts", "runtime-manifest.json"), "utf8"),
  );
  const browser = manifest.camoufox_browser.split("/").at(-1);
  const runtimeId = `py${python.version.major}${python.version.minor}-camoufox-${manifest.camoufox_package}-browser-${browser}`;
  const runtimes = path.join(cacheRoot(platform, env, home), "runtimes");
  return {
    runtimeId,
    pointer: path.join(runtimes, "current.json"),
    defaultDirectory: path.join(runtimes, runtimeId),
  };
}

export function runtimeReady(options) {
  const expected = expectedRuntime(options);
  let pointer;
  try {
    pointer = JSON.parse(fs.readFileSync(expected.pointer, "utf8"));
  } catch {
    return false;
  }
  if (pointer.runtime_id !== expected.runtimeId || !pointer.path) {
    return false;
  }
  const executable = path.join(
    pointer.path,
    options.platform === "win32" ? "Scripts" : "bin",
    options.platform === "win32" ? "python.exe" : "python",
  );
  return fs.existsSync(executable);
}

export function runPython(python, script, args, options = {}) {
  return spawnSync(python.command, [...python.prefix, script, ...args], {
    cwd: options.cwd || process.cwd(),
    env: options.env || process.env,
    shell: false,
    stdio: "inherit",
    windowsHide: true,
  });
}
