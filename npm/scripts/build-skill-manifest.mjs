import crypto from "node:crypto";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const packageRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..", "..");
const skillRoot = path.join(packageRoot, "doudou");
const output = path.join(packageRoot, "npm", "skill-manifest.json");
const allowedRoots = ["SKILL.md", "agents", "references", "scripts"];
const textExtensions = new Set([".json", ".lock", ".md", ".py", ".txt", ".yaml", ".yml"]);

function sha256(value) {
  return crypto.createHash("sha256").update(value).digest("hex");
}

function canonicalContent(filename, content) {
  if (!textExtensions.has(path.extname(filename).toLowerCase())) {
    return content;
  }
  return Buffer.from(content.toString("utf8").replaceAll("\r\n", "\n"), "utf8");
}

function collect(current, root = current) {
  const files = [];
  for (const entry of fs.readdirSync(current, { withFileTypes: true })) {
    if (entry.name === "__pycache__" || entry.name.endsWith(".pyc")) {
      continue;
    }
    const absolute = path.join(current, entry.name);
    if (entry.isDirectory()) {
      files.push(...collect(absolute, root));
    } else if (entry.isFile()) {
      const content = canonicalContent(absolute, fs.readFileSync(absolute));
      files.push({
        path: path.relative(root, absolute).split(path.sep).join("/"),
        bytes: content.length,
        sha256: sha256(content),
      });
    }
  }
  return files;
}

const files = [];
for (const relative of allowedRoots) {
  const absolute = path.join(skillRoot, relative);
  if (!fs.existsSync(absolute)) {
    throw new Error(`Required Skill path is missing: ${relative}`);
  }
  const stat = fs.statSync(absolute);
  if (stat.isDirectory()) {
    files.push(...collect(absolute, skillRoot));
  } else {
    const content = canonicalContent(absolute, fs.readFileSync(absolute));
    files.push({ path: relative, bytes: content.length, sha256: sha256(content) });
  }
}
files.sort((left, right) => left.path.localeCompare(right.path));

const packageRecord = JSON.parse(fs.readFileSync(path.join(packageRoot, "package.json"), "utf8"));
const versionSource = fs.readFileSync(
  path.join(skillRoot, "scripts", "doudou", "__init__.py"),
  "utf8",
);
const versionMatch = versionSource.match(/__version__\s*=\s*["']([^"']+)["']/);
if (!versionMatch) {
  throw new Error("The Skill version could not be read.");
}
if (versionMatch[1] !== packageRecord.version) {
  throw new Error(
    `Skill version ${versionMatch[1]} does not match package version ${packageRecord.version}.`,
  );
}

const treeSha256 = sha256(
  files.map((entry) => `${entry.path}\0${entry.bytes}\0${entry.sha256}\n`).join(""),
);
const manifest = {
  schema: "doudou.npm-skill-manifest.v1",
  package_name: packageRecord.name,
  package_version: packageRecord.version,
  skill_version: versionMatch[1],
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
