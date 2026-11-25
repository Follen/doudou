import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

import { ensureBinary } from "./lib/binary.mjs";
import { recordManagedSkill } from "./lib/managed.mjs";
import { resolveDoudouHome, resolveSkillDirectory } from "./lib/options.mjs";
import { ensureSkill } from "./lib/skill-installer.mjs";

if (process.env.DOUDOU_SKIP_POSTINSTALL === "1") {
  process.exit(0);
}

const packageRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const packageVersion = JSON.parse(
  fs.readFileSync(path.join(packageRoot, "package.json"), "utf8"),
).version;

const target = resolveSkillDirectory();
const doudouHome = resolveDoudouHome();

try {
  await ensureSkill({ packageRoot, target });
  recordManagedSkill(target, doudouHome);
} catch (error) {
  process.stderr.write(`doudou postinstall warning (skill): ${error.message}\n`);
}

try {
  await ensureBinary({ version: packageVersion, doudouHome });
} catch (error) {
  process.stderr.write(
    `doudou postinstall note (binary will be fetched on first run): ${error.message}\n`,
  );
}
