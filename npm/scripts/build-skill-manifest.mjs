import crypto from "node:crypto";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const packageRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..", "..");
const skillRoot = path.join(packageRoot, "doudou");
const output = path.join(packageRoot, "npm", "skill-manifest.json");
const TEXT_EXTENSIONS = new Set([".json", ".md", ".txt", ".yaml", ".yml"]);

function sha256(content) {
  return crypto.createHash("sha256").update(content).digest("hex");
}

function canonicalContent(filename, content) {
  if (!TEXT_EXTENSIONS.has(path.extname(filename).toLowerCase())) {
    return content;
  }
  return Buffer.from(content.toString("utf8").replaceAll("\r\n", "\n"), "utf8");
}

function walkFiles(root, current = root) {
  const results = [];
  for (const entry of fs.readdirSync(current, { withFileTypes: true })) {
    if (entry.name === ".gitignore" || entry.name === ".npmignore") continue;
    const absolute = path.join(current, entry.name);
    if (entry.isDirectory()) {
      results.push(...walkFiles(root, absolute));
    } else if (entry.isFile()) {
      const content = canonicalContent(absolute, fs.readFileSync(absolute));
      results.push({
        path: path.relative(root, absolute).split(path.sep).join("/"),
        bytes: content.length,
        sha256: sha256(content),
      });
    }
  }
  return results.sort((a, b) => a.path.localeCompare(b.path));
}

function readCargoWorkspaceVersion(root) {
  const cargoTomlPath = path.join(root, "Cargo.toml");
  if (!fs.existsSync(cargoTomlPath)) return null;
  const toml = fs.readFileSync(cargoTomlPath, "utf8");
  const section = toml.match(/\[workspace\.package\][\s\S]*?(?=\n\[|$)/);
  const match = section?.[0].match(/^\s*version\s*=\s*"([^"]+)"/m);
  return match?.[1] || null;
}

const packageRecord = JSON.parse(fs.readFileSync(path.join(packageRoot, "package.json"), "utf8"));
const cargoVersion = readCargoWorkspaceVersion(packageRoot);
if (cargoVersion && cargoVersion !== packageRecord.version) {
  throw new Error(
    `Cargo workspace version ${cargoVersion} does not match package.json version ${packageRecord.version}.`,
  );
}

const files = walkFiles(skillRoot);
const treeSha256 = sha256(
  files.map((entry) => `${entry.path}\0${entry.bytes}\0${entry.sha256}\n`).join(""),
);
const manifest = {
  schema: "doudou.npm-skill-manifest.v1",
  package_name: packageRecord.name,
  package_version: packageRecord.version,
  skill_version: packageRecord.version,
  tree_sha256: treeSha256,
  files,
};
const serialized = `${JSON.stringify(manifest, null, 2)}\n`;

if (process.argv.includes("--check")) {
  const current = fs.existsSync(output)
    ? fs.readFileSync(output, "utf8").replaceAll("\r\n", "\n")
    : "";
  if (current !== serialized) {
    process.stderr.write("npm/skill-manifest.json is stale. Run npm run build:manifest.\n");
    process.exitCode = 1;
  }
} else {
  fs.writeFileSync(output, serialized, "utf8");
  process.stdout.write(`Wrote ${path.relative(packageRoot, output)} (${files.length} files).\n`);
}
