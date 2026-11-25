import { spawnSync } from "node:child_process";
import crypto from "node:crypto";
import fs from "node:fs";
import path from "node:path";

const DEFAULT_REPO = "Follen/doudou";
const DOWNLOAD_TIMEOUT_MS = 120_000;

export class BinaryInstallError extends Error {
  constructor(code, message, details = undefined) {
    super(message);
    this.name = "BinaryInstallError";
    this.code = code;
    if (details) this.details = details;
  }
}

export function resolvePlatformSpec(platform = process.platform, arch = process.arch) {
  if (platform === "win32" && arch === "x64") {
    return { target: "x86_64-pc-windows-msvc", ext: "zip", bin: "doudou.exe" };
  }
  if (platform === "linux" && arch === "x64") {
    return { target: "x86_64-unknown-linux-gnu", ext: "tar.gz", bin: "doudou" };
  }
  if (platform === "darwin" && arch === "arm64") {
    return { target: "aarch64-apple-darwin", ext: "tar.gz", bin: "doudou" };
  }
  if (platform === "darwin" && arch === "x64") {
    return { target: "x86_64-apple-darwin", ext: "tar.gz", bin: "doudou" };
  }
  throw new BinaryInstallError(
    "UNSUPPORTED_PLATFORM",
    `No prebuilt doudou binary is available for ${platform}/${arch}. Install from source with \`cargo install --path crates/doudou-cli\`.`,
    { platform, arch },
  );
}

export function assetName(version, spec) {
  return `doudou-v${version}-${spec.target}.${spec.ext}`;
}

export function releaseBaseUrl(version, env = process.env) {
  const override = env.DOUDOU_RELEASE_BASE_URL && env.DOUDOU_RELEASE_BASE_URL.trim();
  if (override) return override.replace(/\/+$/, "");
  return `https://github.com/${DEFAULT_REPO}/releases/download/v${version}`;
}

export function cachedBinaryPath(doudouHome, version, spec) {
  return path.join(doudouHome, "bin", `v${version}`, spec.bin);
}

export function parseChecksum(checksumsText, filename) {
  for (const rawLine of checksumsText.split(/\r?\n/)) {
    const line = rawLine.trim();
    if (!line) continue;
    const match = line.match(/^([0-9a-fA-F]{64})\s+\*?(.+)$/);
    if (!match) continue;
    const [, digest, entryName] = match;
    if (entryName.trim() === filename) {
      return digest.toLowerCase();
    }
  }
  return null;
}

function sha256Buffer(buffer) {
  return crypto.createHash("sha256").update(buffer).digest("hex");
}

async function fetchBuffer(url, fetchImpl = globalThis.fetch) {
  const controller = new AbortController();
  const timer = setTimeout(() => controller.abort(), DOWNLOAD_TIMEOUT_MS);
  try {
    const response = await fetchImpl(url, {
      redirect: "follow",
      signal: controller.signal,
      headers: { "user-agent": "@follenfang/doudou" },
    });
    if (!response.ok) {
      throw new BinaryInstallError(
        "BINARY_DOWNLOAD_FAILED",
        `Failed downloading ${url} (HTTP ${response.status}).`,
        { url, status: response.status },
      );
    }
    return Buffer.from(await response.arrayBuffer());
  } catch (error) {
    if (error instanceof BinaryInstallError) throw error;
    throw new BinaryInstallError(
      "BINARY_DOWNLOAD_FAILED",
      `Failed downloading ${url}: ${error.message}`,
      { url },
    );
  } finally {
    clearTimeout(timer);
  }
}

function extractArchive(archivePath, stagingDir, spec, platform) {
  if (spec.ext === "tar.gz") {
    const result = spawnSync("tar", ["-xzf", archivePath, "-C", stagingDir], {
      encoding: "utf8",
      windowsHide: true,
    });
    if (result.status !== 0) {
      throw new BinaryInstallError(
        "BINARY_EXTRACT_FAILED",
        `Failed extracting ${path.basename(archivePath)}: ${result.stderr || result.error?.message || "tar failed"}`,
      );
    }
    return;
  }

  // .zip on Windows: try built-in bsdtar first, fall back to PowerShell Expand-Archive
  const tarResult = spawnSync("tar", ["-xf", archivePath, "-C", stagingDir], {
    encoding: "utf8",
    windowsHide: true,
  });
  if (tarResult.status === 0) return;

  if (platform === "win32") {
    const ps = spawnSync(
      "powershell",
      [
        "-NoProfile",
        "-NonInteractive",
        "-Command",
        `Expand-Archive -LiteralPath '${archivePath.replaceAll("'", "''")}' -DestinationPath '${stagingDir.replaceAll("'", "''")}' -Force`,
      ],
      { encoding: "utf8", windowsHide: true },
    );
    if (ps.status === 0) return;
    throw new BinaryInstallError(
      "BINARY_EXTRACT_FAILED",
      `Failed extracting ${path.basename(archivePath)}: ${ps.stderr || tarResult.stderr || "extract failed"}`,
    );
  }

  throw new BinaryInstallError(
    "BINARY_EXTRACT_FAILED",
    `Failed extracting ${path.basename(archivePath)}: ${tarResult.stderr || tarResult.error?.message || "extract failed"}`,
  );
}

function findExtractedBinary(root, binName) {
  const direct = path.join(root, binName);
  if (fs.existsSync(direct) && fs.statSync(direct).isFile()) return direct;
  for (const entry of fs.readdirSync(root, { withFileTypes: true })) {
    if (entry.isDirectory()) {
      const nested = path.join(root, entry.name, binName);
      if (fs.existsSync(nested) && fs.statSync(nested).isFile()) return nested;
    }
  }
  return null;
}

export async function ensureBinary({
  version,
  doudouHome,
  platform = process.platform,
  arch = process.arch,
  env = process.env,
  fetchImpl = globalThis.fetch,
}) {
  const explicit = env.DOUDOU_BINARY && env.DOUDOU_BINARY.trim();
  if (explicit) {
    const resolved = path.resolve(explicit);
    if (!fs.existsSync(resolved)) {
      throw new BinaryInstallError(
        "BINARY_NOT_FOUND",
        `DOUDOU_BINARY points to a missing file: ${resolved}`,
        { path: resolved },
      );
    }
    return { path: resolved, source: "env" };
  }

  const spec = resolvePlatformSpec(platform, arch);
  const destination = cachedBinaryPath(doudouHome, version, spec);
  if (fs.existsSync(destination) && fs.statSync(destination).isFile()) {
    return { path: destination, source: "cache" };
  }

  const base = releaseBaseUrl(version, env);
  const archive = assetName(version, spec);
  const checksumsUrl = `${base}/SHA256SUMS.txt`;
  const archiveUrl = `${base}/${archive}`;

  const checksumsBuf = await fetchBuffer(checksumsUrl, fetchImpl);
  const expectedHash = parseChecksum(checksumsBuf.toString("utf8"), archive);
  if (!expectedHash) {
    throw new BinaryInstallError(
      "BINARY_CHECKSUM_MISSING",
      `SHA256SUMS.txt does not contain an entry for ${archive}.`,
      { archive, checksumsUrl },
    );
  }

  const archiveBuf = await fetchBuffer(archiveUrl, fetchImpl);
  const actualHash = sha256Buffer(archiveBuf);
  if (actualHash !== expectedHash) {
    throw new BinaryInstallError(
      "BINARY_CHECKSUM_MISMATCH",
      `SHA-256 mismatch for ${archive}: expected ${expectedHash}, got ${actualHash}.`,
      { archive, expected: expectedHash, actual: actualHash },
    );
  }

  const binRoot = path.join(doudouHome, "bin");
  fs.mkdirSync(binRoot, { recursive: true });
  const nonce = `${process.pid}-${crypto.randomBytes(8).toString("hex")}`;
  const staging = path.join(binRoot, `.stage-${nonce}`);
  fs.mkdirSync(staging, { recursive: true });

  try {
    const archivePath = path.join(staging, archive);
    fs.writeFileSync(archivePath, archiveBuf);
    const extractDir = path.join(staging, "out");
    fs.mkdirSync(extractDir, { recursive: true });
    extractArchive(archivePath, extractDir, spec, platform);

    const extracted = findExtractedBinary(extractDir, spec.bin);
    if (!extracted) {
      throw new BinaryInstallError(
        "BINARY_EXTRACT_FAILED",
        `Archive ${archive} did not contain ${spec.bin}.`,
      );
    }
    if (platform !== "win32") {
      fs.chmodSync(extracted, 0o755);
    }

    const versionDir = path.dirname(destination);
    fs.mkdirSync(versionDir, { recursive: true });
    const tempDest = path.join(versionDir, `.${spec.bin}.${nonce}.tmp`);
    fs.copyFileSync(extracted, tempDest);
    if (platform !== "win32") {
      fs.chmodSync(tempDest, 0o755);
    }
    try {
      fs.renameSync(tempDest, destination);
    } catch (error) {
      fs.rmSync(tempDest, { force: true });
      if (!fs.existsSync(destination)) throw error;
    }
    fs.writeFileSync(path.join(versionDir, `${spec.bin}.sha256`), `${actualHash}  ${archive}\n`, "utf8");
    return { path: destination, source: "downloaded", sha256: actualHash };
  } finally {
    fs.rmSync(staging, { recursive: true, force: true });
  }
}
