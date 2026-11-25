import { spawnSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const testDir = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..", "test");
const files = fs
  .readdirSync(testDir)
  .filter((name) => name.endsWith(".test.mjs"))
  .sort()
  .map((name) => path.join(testDir, name));

const result = spawnSync(process.execPath, ["--test", ...files], {
  stdio: "inherit",
  windowsHide: true,
});
process.exit(result.status ?? 1);
