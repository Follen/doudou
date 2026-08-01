"use strict";

const crypto = require("node:crypto");
const fs = require("node:fs");
const fsp = require("node:fs/promises");
const path = require("node:path");

class AppError extends Error {
  constructor(code, message, details = null, exitCode = 1) {
    super(message);
    this.name = "AppError";
    this.code = code;
    this.details = details;
    this.exitCode = exitCode;
  }
}

function parseArgs(argv) {
  const args = { _: [] };
  for (let index = 0; index < argv.length; index += 1) {
    const token = argv[index];
    if (!token.startsWith("--")) {
      args._.push(token);
      continue;
    }
    const separator = token.indexOf("=");
    const key = (separator === -1 ? token.slice(2) : token.slice(2, separator)).replaceAll("-", "_");
    if (separator !== -1) {
      const value = token.slice(separator + 1);
      if (key === "at" && args[key] !== undefined) args[key] = [].concat(args[key], value);
      else args[key] = value;
      continue;
    }
    const next = argv[index + 1];
    if (next !== undefined && !next.startsWith("--")) {
      if (key === "at" && args[key] !== undefined) args[key] = [].concat(args[key], next);
      else args[key] = next;
      index += 1;
    } else {
      args[key] = true;
    }
  }
  return args;
}

function emit(event, payload = {}) {
  process.stdout.write(`${JSON.stringify({ event, ...payload })}\n`);
}

function now() {
  return new Date().toISOString();
}

function sha256(value) {
  return crypto.createHash("sha256").update(value).digest("hex");
}

function randomId(prefix) {
  return `${prefix}-${new Date().toISOString().replace(/[-:.TZ]/g, "").slice(0, 14)}-${crypto.randomUUID().slice(0, 8)}`;
}

function safeSegment(value, maxLength = 96) {
  const text = String(value ?? "").normalize("NFKC").trim();
  if (!text) {
    throw new AppError("INVALID_PATH_SEGMENT", "A non-empty path value is required.");
  }
  let encoded = "";
  for (const character of text) {
    if (/[A-Za-z0-9._-]/.test(character)) {
      encoded += character;
    } else if (/^[\p{L}\p{N}]$/u.test(character)) {
      encoded += character;
    } else {
      encoded += `~${character.codePointAt(0).toString(16).toUpperCase().padStart(4, "0")}`;
    }
  }
  encoded = encoded.replace(/^\.+$/, (match) => `~${Buffer.from(match).toString("hex")}`);
  if (encoded.length > maxLength) {
    encoded = `${encoded.slice(0, maxLength - 13)}~${sha256(text).slice(0, 12)}`;
  }
  return encoded;
}

function resolveDataRoot(candidate) {
  return path.resolve(candidate || process.env.DOUDOU_HOME || process.cwd());
}

async function ensureDir(directory) {
  await fsp.mkdir(directory, { recursive: true });
  return directory;
}

async function atomicWriteFile(filename, content) {
  await ensureDir(path.dirname(filename));
  const temporary = path.join(path.dirname(filename), `.${path.basename(filename)}.${process.pid}.${crypto.randomUUID()}.tmp`);
  try {
    await fsp.writeFile(temporary, content, "utf8");
    await fsp.rename(temporary, filename);
  } catch (error) {
    await fsp.rm(temporary, { force: true }).catch(() => {});
    throw error;
  }
}

async function atomicWriteJson(filename, value) {
  await atomicWriteFile(filename, `${JSON.stringify(value, null, 2)}\n`);
}

async function readJson(filename, { optional = false } = {}) {
  try {
    return JSON.parse(await fsp.readFile(filename, "utf8"));
  } catch (error) {
    if (optional && error.code === "ENOENT") return null;
    if (error instanceof SyntaxError) {
      throw new AppError("INVALID_JSON", `Invalid JSON in ${filename}.`, { path: filename, message: error.message });
    }
    throw error;
  }
}

async function readJsonl(filename, { optional = false, validate = null } = {}) {
  let content;
  try {
    content = await fsp.readFile(filename, "utf8");
  } catch (error) {
    if (optional && error.code === "ENOENT") return [];
    throw error;
  }
  if (content && !content.endsWith("\n")) {
    throw new AppError("INCOMPLETE_JSONL_LINE", `JSONL file has an incomplete final line: ${filename}.`, { path: filename });
  }
  const records = [];
  const lines = content.split(/\r?\n/);
  for (let index = 0; index < lines.length; index += 1) {
    if (!lines[index]) continue;
    let value;
    try {
      value = JSON.parse(lines[index]);
    } catch (error) {
      throw new AppError("INVALID_JSONL", `Invalid JSONL at ${filename}:${index + 1}.`, { path: filename, line: index + 1, message: error.message });
    }
    if (validate) validate(value, { path: filename, line: index + 1 });
    records.push(value);
  }
  return records;
}

async function atomicWriteJsonl(filename, records) {
  const content = records.length ? `${records.map((record) => JSON.stringify(record)).join("\n")}\n` : "";
  await atomicWriteFile(filename, content);
}

async function appendJsonl(filename, record) {
  await ensureDir(path.dirname(filename));
  await fsp.appendFile(filename, `${JSON.stringify(record)}\n`, "utf8");
}

function exists(filename) {
  return fs.existsSync(filename);
}

function sleep(milliseconds) {
  return new Promise((resolve) => setTimeout(resolve, milliseconds));
}

function requireString(args, key) {
  const value = args[key];
  if (typeof value !== "string" || !value.trim()) {
    throw new AppError("MISSING_ARGUMENT", `Missing required argument --${key.replaceAll("_", "-")}.`, { argument: key });
  }
  return value.trim();
}

function parseTimestamp(value, name) {
  const milliseconds = Date.parse(value);
  if (!Number.isFinite(milliseconds)) {
    throw new AppError("INVALID_TIMESTAMP", `${name} must be an RFC3339 timestamp.`, { argument: name, value });
  }
  return new Date(milliseconds).toISOString();
}

module.exports = {
  AppError,
  appendJsonl,
  atomicWriteFile,
  atomicWriteJson,
  atomicWriteJsonl,
  emit,
  ensureDir,
  exists,
  now,
  parseArgs,
  parseTimestamp,
  randomId,
  readJson,
  readJsonl,
  requireString,
  resolveDataRoot,
  resolveProjectRoot: resolveDataRoot,
  safeSegment,
  sha256,
  sleep,
};
