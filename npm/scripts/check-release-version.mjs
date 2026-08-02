import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const packageRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..", "..");
const packageRecord = JSON.parse(fs.readFileSync(path.join(packageRoot, "package.json"), "utf8"));
const source = fs.readFileSync(
  path.join(packageRoot, "doudou", "scripts", "doudou", "__init__.py"),
  "utf8",
);
const match = source.match(/__version__\s*=\s*["']([^"']+)["']/);
const tag = process.env.GITHUB_REF_NAME || process.argv[2];
if (!tag || !tag.startsWith("v")) {
  throw new Error("A v-prefixed release tag is required.");
}
const tagVersion = tag.slice(1);
if (!match || packageRecord.version !== tagVersion || match[1] !== tagVersion) {
  throw new Error(
    `Version mismatch: tag=${tagVersion}, package=${packageRecord.version}, skill=${match?.[1] || "missing"}.`,
  );
}
process.stdout.write(`Release version ${tagVersion} is consistent.\n`);
