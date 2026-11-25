import os from "node:os";
import path from "node:path";

function expandTilde(raw, home) {
  if (!raw) return raw;
  const userHome = home || os.homedir();
  if (raw === "~") return userHome;
  if (raw.startsWith("~/") || raw.startsWith("~\\")) {
    return path.join(userHome, raw.slice(2));
  }
  return raw;
}

export function resolveSkillDirectory({ explicit, env = process.env, home } = {}) {
  const userHome = home || os.homedir();
  if (explicit) {
    return path.resolve(expandTilde(explicit, userHome));
  }
  if (env.DOUDOU_AGENT_HOME && env.DOUDOU_AGENT_HOME.trim()) {
    return path.resolve(expandTilde(env.DOUDOU_AGENT_HOME.trim(), userHome), "skills", "doudou");
  }
  return path.join(userHome, ".agents", "skills", "doudou");
}

export function resolveDoudouHome({
  explicit,
  env = process.env,
  platform = process.platform,
  home,
} = {}) {
  const userHome = home || os.homedir();
  if (explicit) {
    return path.resolve(expandTilde(explicit, userHome));
  }
  if (env.DOUDOU_HOME && env.DOUDOU_HOME.trim()) {
    const candidate = expandTilde(env.DOUDOU_HOME.trim(), userHome);
    return path.isAbsolute(candidate) ? path.resolve(candidate) : path.resolve(userHome, candidate);
  }
  if (platform === "win32") {
    const localAppData =
      (env.LOCALAPPDATA && env.LOCALAPPDATA.trim()) || path.join(userHome, "AppData", "Local");
    return path.resolve(localAppData, "doudou");
  }
  if (platform === "darwin") {
    return path.resolve(userHome, "Library", "Application Support", "doudou");
  }
  const xdgDataHome =
    (env.XDG_DATA_HOME && env.XDG_DATA_HOME.trim()) || path.join(userHome, ".local", "share");
  return path.resolve(xdgDataHome, "doudou");
}

export function parseLauncherOptions(argv) {
  const forwarded = [];
  let skillDirectory;
  let homeDirectory;

  for (let index = 0; index < argv.length; index += 1) {
    const value = argv[index];
    if (value === "--skill-dir") {
      const candidate = argv[index + 1];
      if (!candidate || candidate.startsWith("--")) {
        throw new Error("--skill-dir requires a directory path.");
      }
      skillDirectory = candidate;
      index += 1;
      continue;
    }
    if (value.startsWith("--skill-dir=")) {
      skillDirectory = value.slice("--skill-dir=".length);
      if (!skillDirectory) {
        throw new Error("--skill-dir requires a directory path.");
      }
      continue;
    }
    if (value === "--home" && argv[index + 1] && !argv[index + 1].startsWith("--")) {
      homeDirectory = argv[index + 1];
    } else if (value.startsWith("--home=") && value.length > "--home=".length) {
      homeDirectory = value.slice("--home=".length);
    }
    forwarded.push(value);
  }
  return { forwarded, skillDirectory, homeDirectory };
}

export function firstCommand(argv) {
  for (let index = 0; index < argv.length; index += 1) {
    const value = argv[index];
    if (value === "--home" || value === "--data-root") {
      index += 1;
      continue;
    }
    if (value.startsWith("--home=") || value.startsWith("--data-root=") || value.startsWith("-")) {
      continue;
    }
    return value;
  }
  return undefined;
}
