import path from "node:path";
import { fileURLToPath } from "node:url";

import { resolveSkillDirectory } from "./lib/options.mjs";
import { ensureSkill } from "./lib/skill-installer.mjs";

const packageRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");

try {
  const target = resolveSkillDirectory();
  const result = await ensureSkill({ packageRoot, target });
  process.stdout.write(`doudou Skill ${result.status}: ${target}\n`);
} catch (error) {
  process.stderr.write(
    `doudou postinstall skipped: ${error.message}. The doudou command will retry.\n`,
  );
}
