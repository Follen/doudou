import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import crypto from "node:crypto";
import fs from "node:fs";
import http from "node:http";
import os from "node:os";
import path from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

import {
  assetName,
  ensureBinary,
  parseChecksum,
  resolvePlatformSpec,
} from "../lib/binary.mjs";
import {
  firstCommand,
  parseLauncherOptions,
  resolveDoudouHome,
  resolveSkillDirectory,
} from "../lib/options.mjs";
import { ensureSkill, INSTALL_STAMP } from "../lib/skill-installer.mjs";

const packageRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..", "..");

function makeTempDir(prefix) {
  return fs.mkdtempSync(path.join(os.tmpdir(), prefix));
}

function createArchive(sourceDir, binName, archivePath, ext) {
  if (ext === "tar.gz") {
    const res = spawnSync("tar", ["-czf", archivePath, "-C", sourceDir, binName], {
      encoding: "utf8",
      windowsHide: true,
    });
    assert.equal(res.status, 0, res.stderr);
    return;
  }
  const tarRes = spawnSync("tar", ["-a", "-cf", archivePath, "-C", sourceDir, binName], {
    encoding: "utf8",
    windowsHide: true,
  });
  if (tarRes.status === 0) return;
  const psRes = spawnSync(
    "powershell",
    [
      "-NoProfile",
      "-NonInteractive",
      "-Command",
      `Compress-Archive -LiteralPath '${path.join(sourceDir, binName).replaceAll("'", "''")}' -DestinationPath '${archivePath.replaceAll("'", "''")}' -Force`,
    ],
    { encoding: "utf8", windowsHide: true },
  );
  assert.equal(psRes.status, 0, psRes.stderr || tarRes.stderr);
}

test("resolveDoudouHome matches Rust AppData priority across OSes", () => {
  assert.equal(
    resolveDoudouHome({
      platform: "win32",
      env: { LOCALAPPDATA: "C:\\Users\\demo\\AppData\\Local" },
      home: "C:\\Users\\demo",
    }),
    path.resolve("C:\\Users\\demo\\AppData\\Local", "doudou"),
  );
  assert.equal(
    resolveDoudouHome({
      platform: "darwin",
      env: {},
      home: "/Users/demo",
    }),
    path.resolve("/Users/demo", "Library", "Application Support", "doudou"),
  );
  assert.equal(
    resolveDoudouHome({
      platform: "linux",
      env: { XDG_DATA_HOME: "/tmp/xdg" },
      home: "/home/demo",
    }),
    path.resolve("/tmp/xdg", "doudou"),
  );
  assert.equal(
    resolveDoudouHome({
      explicit: "/custom/home",
      env: { DOUDOU_HOME: "/env/home" },
      platform: "linux",
      home: "/home/demo",
    }),
    path.resolve("/custom/home"),
  );
});

test("parseLauncherOptions extracts --skill-dir while forwarding --home", () => {
  const parsed = parseLauncherOptions([
    "--skill-dir",
    "/tmp/skill",
    "--home",
    "/tmp/home",
    "doctor",
    "--launch-probe",
  ]);
  assert.equal(parsed.skillDirectory, "/tmp/skill");
  assert.equal(parsed.homeDirectory, "/tmp/home");
  assert.deepEqual(parsed.forwarded, ["--home", "/tmp/home", "doctor", "--launch-probe"]);
  assert.equal(firstCommand(parsed.forwarded), "doctor");

  assert.equal(
    resolveSkillDirectory({ env: { DOUDOU_AGENT_HOME: "/tmp/agent" } }),
    path.resolve("/tmp/agent", "skills", "doudou"),
  );
});

test("ensureSkill installs atomically, is idempotent, and protects unmanaged targets", async () => {
  const temp = makeTempDir("doudou-skill-test-");
  try {
    const target = path.join(temp, "skills", "doudou");
    const first = await ensureSkill({ packageRoot, target });
    assert.equal(first.status, "installed");
    assert.ok(fs.existsSync(path.join(target, "SKILL.md")));
    assert.ok(fs.existsSync(path.join(target, "agents", "openai.yaml")));
    assert.ok(fs.existsSync(path.join(target, INSTALL_STAMP)));

    const second = await ensureSkill({ packageRoot, target });
    assert.equal(second.status, "current");

    const unmanaged = path.join(temp, "unmanaged");
    fs.mkdirSync(unmanaged, { recursive: true });
    fs.writeFileSync(path.join(unmanaged, "custom.txt"), "do not overwrite\n");
    await assert.rejects(() => ensureSkill({ packageRoot, target: unmanaged }), /not a matching managed/);
  } finally {
    fs.rmSync(temp, { recursive: true, force: true });
  }
});

test("ensureBinary downloads, verifies SHA-256, caches, and rejects tampered archives", async () => {
  const temp = makeTempDir("doudou-bin-test-");
  const spec = resolvePlatformSpec();
  const version = "0.1.0";
  const archiveFile = assetName(version, spec);

  const srcDir = path.join(temp, "src");
  fs.mkdirSync(srcDir, { recursive: true });
  const fakePayload = "fake-rust-binary-payload-v0.1.0";
  fs.writeFileSync(path.join(srcDir, spec.bin), fakePayload);

  const archivePath = path.join(temp, archiveFile);
  createArchive(srcDir, spec.bin, archivePath, spec.ext);
  const archiveBytes = fs.readFileSync(archivePath);
  const goodHash = crypto.createHash("sha256").update(archiveBytes).digest("hex");

  assert.equal(parseChecksum(`${goodHash}  ${archiveFile}\n`, archiveFile), goodHash);

  let mode = "valid";
  let hits = 0;
  const server = http.createServer((req, res) => {
    hits += 1;
    if (req.url === "/SHA256SUMS.txt") {
      const digest = mode === "valid" ? goodHash : "0".repeat(64);
      res.writeHead(200, { "content-type": "text/plain" });
      res.end(`${digest}  ${archiveFile}\n`);
      return;
    }
    if (req.url === `/${archiveFile}`) {
      res.writeHead(200, { "content-type": "application/octet-stream" });
      res.end(archiveBytes);
      return;
    }
    res.writeHead(404);
    res.end("not found");
  });

  await new Promise((resolve) => server.listen(0, "127.0.0.1", resolve));
  const { port } = server.address();
  const baseUrl = `http://127.0.0.1:${port}`;

  try {
    // 1. Tampered checksum must fail closed
    mode = "bad";
    const badHome = path.join(temp, "bad-home");
    await assert.rejects(
      () =>
        ensureBinary({
          version,
          doudouHome: badHome,
          env: { DOUDOU_RELEASE_BASE_URL: baseUrl },
        }),
      (err) => err.code === "BINARY_CHECKSUM_MISMATCH",
    );

    // 2. Valid checksum downloads and extracts
    mode = "valid";
    hits = 0;
    const goodHome = path.join(temp, "good-home");
    const first = await ensureBinary({
      version,
      doudouHome: goodHome,
      env: { DOUDOU_RELEASE_BASE_URL: baseUrl },
    });
    assert.equal(first.source, "downloaded");
    assert.equal(fs.readFileSync(first.path, "utf8"), fakePayload);
    assert.equal(hits, 2);

    // 3. Second call reuses cache with zero network requests
    const second = await ensureBinary({
      version,
      doudouHome: goodHome,
      env: { DOUDOU_RELEASE_BASE_URL: baseUrl },
    });
    assert.equal(second.source, "cache");
    assert.equal(second.path, first.path);
    assert.equal(hits, 2);
  } finally {
    server.close();
    fs.rmSync(temp, { recursive: true, force: true });
  }
});

test("launcher CLI installs skill and prints version cleanly", () => {
  const temp = makeTempDir("doudou-cli-launcher-");
  try {
    const skillDir = path.join(temp, "skill");
    const homeDir = path.join(temp, "home");
    const res = spawnSync(
      process.execPath,
      [
        path.join(packageRoot, "npm", "bin", "doudou.mjs"),
        "--skill-dir",
        skillDir,
        "--version",
      ],
      {
        encoding: "utf8",
        env: { ...process.env, DOUDOU_HOME: homeDir },
        windowsHide: true,
      },
    );
    assert.equal(res.status, 0, res.stderr);
    assert.match(res.stdout.trim(), /^doudou \d+\.\d+\.\d+/);
    assert.ok(fs.existsSync(path.join(skillDir, "SKILL.md")));
  } finally {
    fs.rmSync(temp, { recursive: true, force: true });
  }
});
