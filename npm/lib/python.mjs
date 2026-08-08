import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { spawnSync } from "node:child_process";

export function doudouCacheRoot({ env = process.env, home = os.homedir() } = {}) {
  const configured = env.DOUDOU_HOME;
  if (!configured) return path.join(home, ".doudou");
  if (configured === "~") return path.resolve(home);
  if (configured.startsWith("~/") || configured.startsWith("~\\")) {
    return path.resolve(home, configured.slice(2));
  }
  return path.resolve(path.isAbsolute(configured) ? configured : path.join(home, configured));
}

export function assertCurrentRuntimeLayout({ platform = process.platform, env = process.env, home = os.homedir() } = {}) {
  const current = doudouCacheRoot({ env, home });
  const unsupported = platform === "win32"
    ? path.join(env.LOCALAPPDATA || path.join(home, "AppData", "Local"), "doudou")
    : path.join(env.XDG_CACHE_HOME || path.join(home, ".cache"), "doudou");
  if (path.resolve(unsupported) === path.resolve(current) || !fs.existsSync(unsupported)) return;
  if (fs.lstatSync(unsupported).isSymbolicLink()) return;
  const markers = [
    path.join(unsupported, "runtimes", "current.json"),
    path.join(unsupported, "setup-state.json"),
  ];
  if (!markers.some((filename) => fs.existsSync(filename))) return;
  const error = new Error("An unsupported Doudou runtime layout was found. Remove or relocate it before using the current runtime.");
  error.code = "LEGACY_UNSUPPORTED";
  error.details = { legacy_kind: "runtime_root", supported_root: current };
  throw error;
}

export function currentRuntime({ platform = process.platform, env = process.env, home = os.homedir() } = {}) {
  const pointer = path.join(doudouCacheRoot({ platform, env, home }), "runtimes", "current.json");
  try {
    const value = JSON.parse(fs.readFileSync(pointer, "utf8"));
    if (!value.path || !value.runtime_id) return null;
    const executable = path.join(value.path, "venv", platform === "win32" ? "Scripts/python.exe" : "bin/python");
    return fs.existsSync(executable) ? { ...value, executable } : null;
  } catch {
    return null;
  }
}

export function runPython(python, script, args, options = {}) {
  const executable = typeof python === "string" ? python : python.executable;
  return spawnSync(executable, [script, ...args], {
    cwd: options.cwd || process.cwd(),
    env: options.env || process.env,
    shell: false,
    stdio: "inherit",
    windowsHide: true,
  });
}
