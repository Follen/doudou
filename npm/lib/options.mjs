import os from "node:os";
import path from "node:path";

const CLI_COMMANDS = new Set([
  "doctor",
  "setup",
  "uninstall",
  "profile",
  "broker",
  "session",
  "conversations",
  "messages",
  "protocol",
  "chat",
  "identity",
  "videos",
  "video",
  "page-doctor",
]);

export function resolveSkillDirectory({ explicit, env = process.env, home } = {}) {
  if (explicit) {
    return path.resolve(explicit);
  }
  if (env.DOUDOU_AGENT_HOME) {
    return path.resolve(env.DOUDOU_AGENT_HOME, "skills", "doudou");
  }
  return path.join(home || os.homedir(), ".agents", "skills", "doudou");
}

export function parseLauncherOptions(argv) {
  const forwarded = [];
  let skillDirectory;
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
    forwarded.push(value);
  }
  return { forwarded, skillDirectory };
}

export function detectCliCommand(argv) {
  for (let index = 0; index < argv.length; index += 1) {
    const value = argv[index];
    if (value === "--data-root") {
      index += 1;
      continue;
    }
    if (value.startsWith("--data-root=") || value.startsWith("-")) {
      continue;
    }
    return CLI_COMMANDS.has(value) ? value : undefined;
  }
  return undefined;
}
