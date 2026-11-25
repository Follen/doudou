import crypto from "node:crypto";
import fs from "node:fs";
import path from "node:path";

export const INSTALL_STAMP = ".doudou-npm-install.json";
export const INSTALL_STAMP_SCHEMA = "doudou.npm-skill-install.v1";
const LOCK_STALE_MS = 5 * 60 * 1000;
const LOCK_WAIT_MS = 15 * 1000;
const TEXT_EXTENSIONS = new Set([".json", ".md", ".txt", ".yaml", ".yml"]);

function sha256(buffer) {
  return crypto.createHash("sha256").update(buffer).digest("hex");
}

function canonicalContent(filename, content) {
  if (!TEXT_EXTENSIONS.has(path.extname(filename).toLowerCase())) {
    return content;
  }
  return Buffer.from(content.toString("utf8").replaceAll("\r\n", "\n"), "utf8");
}

function sleep(ms) {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

function readJson(file) {
  return JSON.parse(fs.readFileSync(file, "utf8"));
}

function writeJson(file, value) {
  fs.writeFileSync(file, `${JSON.stringify(value, null, 2)}\n`, "utf8");
}

export function normalizedFiles(root, current = root) {
  if (!fs.existsSync(current)) return [];
  const result = [];
  for (const entry of fs.readdirSync(current, { withFileTypes: true })) {
    if (
      entry.name === INSTALL_STAMP ||
      entry.name === ".gitignore" ||
      entry.name === ".npmignore"
    ) {
      continue;
    }
    const absolute = path.join(current, entry.name);
    if (entry.isDirectory()) {
      result.push(...normalizedFiles(root, absolute));
    } else if (entry.isFile()) {
      result.push(path.relative(root, absolute).split(path.sep).join("/"));
    }
  }
  return result.sort((a, b) => a.localeCompare(b));
}

export function loadDistribution(packageRoot) {
  const packageRecord = readJson(path.join(packageRoot, "package.json"));
  const manifest = readJson(path.join(packageRoot, "npm", "skill-manifest.json"));
  if (
    manifest.schema !== "doudou.npm-skill-manifest.v1" ||
    manifest.package_name !== packageRecord.name ||
    manifest.package_version !== packageRecord.version
  ) {
    throw new Error("The packaged Skill manifest does not match package.json.");
  }
  return {
    packageRecord,
    manifest,
    sourceSkill: path.join(packageRoot, "doudou"),
  };
}

export function verifySkill(root, manifest) {
  const expected = manifest.files.map((entry) => entry.path);
  const actual = normalizedFiles(root);
  if (JSON.stringify(actual) !== JSON.stringify(expected)) {
    return { valid: false, reason: "file_inventory_mismatch" };
  }
  for (const entry of manifest.files) {
    const absolute = path.join(root, ...entry.path.split("/"));
    let content;
    try {
      content = canonicalContent(absolute, fs.readFileSync(absolute));
    } catch {
      return { valid: false, reason: "file_missing", path: entry.path };
    }
    if (content.length !== entry.bytes || sha256(content) !== entry.sha256) {
      return { valid: false, reason: "content_mismatch", path: entry.path };
    }
  }
  return { valid: true };
}

function managedStamp(target) {
  const stampPath = path.join(target, INSTALL_STAMP);
  if (!fs.existsSync(stampPath)) return null;
  try {
    const value = readJson(stampPath);
    if (value?.schema === INSTALL_STAMP_SCHEMA) return value;
  } catch {
    return null;
  }
  return null;
}

function installStamp(distribution, target) {
  return {
    schema: INSTALL_STAMP_SCHEMA,
    package_name: distribution.packageRecord.name,
    package_version: distribution.packageRecord.version,
    skill_version: distribution.manifest.skill_version,
    tree_sha256: distribution.manifest.tree_sha256,
    target: path.resolve(target),
    installed_at: new Date().toISOString(),
  };
}

function copyManifestFiles(sourceRoot, destinationRoot, manifest) {
  fs.mkdirSync(destinationRoot, { recursive: true });
  for (const entry of manifest.files) {
    const source = path.join(sourceRoot, ...entry.path.split("/"));
    const destination = path.join(destinationRoot, ...entry.path.split("/"));
    fs.mkdirSync(path.dirname(destination), { recursive: true });
    fs.writeFileSync(destination, canonicalContent(source, fs.readFileSync(source)));
  }
}

async function acquireLock(target) {
  const parent = path.dirname(target);
  fs.mkdirSync(parent, { recursive: true });
  const lockPath = path.join(parent, `.${path.basename(target)}.doudou-install.lock`);
  const deadline = Date.now() + LOCK_WAIT_MS;
  while (true) {
    try {
      const fd = fs.openSync(lockPath, "wx");
      fs.writeFileSync(fd, JSON.stringify({ pid: process.pid, created_at: Date.now() }));
      fs.closeSync(fd);
      return {
        release() {
          fs.rmSync(lockPath, { force: true });
        },
      };
    } catch (error) {
      if (error.code !== "EEXIST") throw error;
      try {
        const stat = fs.statSync(lockPath);
        if (Date.now() - stat.mtimeMs > LOCK_STALE_MS) {
          fs.rmSync(lockPath, { force: true });
          continue;
        }
      } catch {
        continue;
      }
      if (Date.now() >= deadline) {
        throw new Error(`Timed out waiting for Skill installation lock: ${lockPath}`);
      }
      await sleep(100);
    }
  }
}

export async function ensureSkill({ packageRoot, target }) {
  const distribution = loadDistribution(packageRoot);
  const sourceCheck = verifySkill(distribution.sourceSkill, distribution.manifest);
  if (!sourceCheck.valid) {
    throw new Error(`The packaged Skill failed validation: ${sourceCheck.reason}.`);
  }
  const lock = await acquireLock(target);
  try {
    const exists = fs.existsSync(target);
    const targetCheck = exists
      ? verifySkill(target, distribution.manifest)
      : { valid: false, reason: "missing" };
    const stamp = exists ? managedStamp(target) : null;
    if (targetCheck.valid) {
      if (!stamp) {
        writeJson(path.join(target, INSTALL_STAMP), installStamp(distribution, target));
        return { status: "adopted", target, distribution };
      }
      if (stamp.package_name !== distribution.packageRecord.name) {
        throw new Error(`The Skill target is managed by ${stamp.package_name}.`);
      }
      if (stamp.target !== path.resolve(target)) {
        writeJson(path.join(target, INSTALL_STAMP), installStamp(distribution, target));
      }
      return { status: "current", target, distribution };
    }
    if (exists && (!stamp || stamp.package_name !== distribution.packageRecord.name)) {
      throw new Error(
        `The Skill target already exists and is not a matching managed doudou Skill: ${target}`,
      );
    }

    const parent = path.dirname(target);
    const nonce = `${process.pid}-${crypto.randomBytes(8).toString("hex")}`;
    const staging = path.join(parent, `.doudou-stage-${nonce}`);
    const backup = path.join(parent, `.doudou-backup-${nonce}`);
    let movedOld = false;
    try {
      copyManifestFiles(distribution.sourceSkill, staging, distribution.manifest);
      writeJson(path.join(staging, INSTALL_STAMP), installStamp(distribution, target));
      const stagingCheck = verifySkill(staging, distribution.manifest);
      if (!stagingCheck.valid) {
        throw new Error(`The staged Skill failed validation: ${stagingCheck.reason}.`);
      }
      if (exists) {
        fs.renameSync(target, backup);
        movedOld = true;
      }
      try {
        fs.renameSync(staging, target);
      } catch (error) {
        if (movedOld && !fs.existsSync(target)) {
          fs.renameSync(backup, target);
          movedOld = false;
        }
        throw error;
      }
      if (movedOld) {
        fs.rmSync(backup, { recursive: true, force: true });
      }
      return { status: exists ? "upgraded" : "installed", target, distribution };
    } finally {
      fs.rmSync(staging, { recursive: true, force: true });
      if (movedOld && fs.existsSync(backup) && !fs.existsSync(target)) {
        fs.renameSync(backup, target);
      }
    }
  } finally {
    lock.release();
  }
}
