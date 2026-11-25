import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const packageRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..", "..");
const packageRecord = JSON.parse(fs.readFileSync(path.join(packageRoot, "package.json"), "utf8"));
const manifest = JSON.parse(
  fs.readFileSync(path.join(packageRoot, "npm", "skill-manifest.json"), "utf8"),
);
const cargoToml = fs.readFileSync(path.join(packageRoot, "Cargo.toml"), "utf8");
const section = cargoToml.match(/\[workspace\.package\][\s\S]*?(?=\n\[|$)/);
const cargoVersion = section?.[0].match(/^\s*version\s*=\s*"([^"]+)"/m)?.[1];

const tag = process.env.GITHUB_REF_NAME || process.argv[2];
if (!tag || !tag.startsWith("v")) {
  throw new Error("A v-prefixed release tag is required.");
}
const tagVersion = tag.slice(1);
if (
  packageRecord.version !== tagVersion ||
  cargoVersion !== tagVersion ||
  manifest.package_version !== tagVersion ||
  manifest.skill_version !== tagVersion
) {
  throw new Error(
    `Version mismatch: tag=${tagVersion}, package=${packageRecord.version}, cargo=${cargoVersion || "missing"}, manifest=${manifest.package_version}/${manifest.skill_version}.`,
  );
}
process.stdout.write(`Release version ${tagVersion} is consistent across tag, package.json, Cargo.toml, and skill-manifest.json.\n`);
