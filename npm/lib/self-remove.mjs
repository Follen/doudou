import { spawnSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";

const PACKAGE_NAME = "@follenfang/doudou";

export function installationPrefix(packageRoot, pathApi = path) {
  let cursor = pathApi.resolve(packageRoot);
  while (
    pathApi.dirname(cursor) !== cursor &&
    pathApi.basename(cursor) !== "node_modules"
  ) {
    cursor = pathApi.dirname(cursor);
  }
  if (pathApi.basename(cursor) !== "node_modules") {
    throw new Error("The npm installation prefix could not be derived.");
  }
  const parent = pathApi.dirname(cursor);
  return pathApi.basename(parent) === "lib" ? pathApi.dirname(parent) : parent;
}

export function resolveNpmCli({
  platform = process.platform,
  env = process.env,
  run = spawnSync,
} = {}) {
  if (env.npm_execpath && fs.existsSync(env.npm_execpath)) {
    return path.resolve(env.npm_execpath);
  }
  if (platform === "win32") {
    const located = run("where.exe", ["npm.cmd"], {
      encoding: "utf8",
      windowsHide: true,
    });
    if (located.status === 0) {
      for (const shim of located.stdout.split(/\r?\n/).filter(Boolean)) {
        const candidate = path.join(
          path.dirname(shim.trim()),
          "node_modules",
          "npm",
          "bin",
          "npm-cli.js",
        );
        if (fs.existsSync(candidate)) {
          return candidate;
        }
      }
    }
  } else {
    const located = run("which", ["npm"], { encoding: "utf8" });
    if (located.status === 0) {
      try {
        const candidate = fs.realpathSync(located.stdout.trim());
        if (fs.existsSync(candidate)) {
          return candidate;
        }
      } catch {
        // Fall through to the stable error below.
      }
    }
  }
  throw new Error("npm-cli.js could not be located for self-removal.");
}

export function removeCurrentNpmPackage({
  packageRoot,
  platform = process.platform,
  env = process.env,
  run = spawnSync,
  npmCli,
} = {}) {
  const prefix = installationPrefix(packageRoot);
  const args = [
    npmCli || resolveNpmCli({ platform, env, run }),
    "uninstall",
    "-g",
    "--ignore-scripts",
    "--prefix",
    prefix,
    PACKAGE_NAME,
  ];
  const result = run(process.execPath, args, {
    encoding: "utf8",
    shell: false,
    windowsHide: true,
  });
  if (result.error || result.status !== 0) {
    const reason = result.error?.message || result.stderr?.trim() || "npm uninstall failed";
    throw new Error(`The npm package could not remove itself: ${reason}`);
  }
  return { package_name: PACKAGE_NAME, prefix, status: "removed" };
}
