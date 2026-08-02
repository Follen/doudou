import assert from "node:assert/strict";
import crypto from "node:crypto";
import fs from "node:fs";
import http from "node:http";
import os from "node:os";
import path from "node:path";
import test from "node:test";

import { controlledDownload, downloadInternals } from "../lib/controlled-download.mjs";

function sha(value) { return crypto.createHash("sha256").update(value).digest("hex"); }

async function server(t, handler) {
  const instance = http.createServer(handler);
  await new Promise((resolve) => instance.listen(0, "127.0.0.1", resolve));
  t.after(() => new Promise((resolve) => instance.close(resolve)));
  return `http://127.0.0.1:${instance.address().port}`;
}

function destination(t) {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "doudou-download-test-"));
  t.after(() => fs.rmSync(root, { recursive: true, force: true }));
  return path.join(root, "asset.bin");
}

function options(filename, overrides = {}) {
  return { destination: filename, identity: "fixture", stage: "test", connectTimeoutMs: 1000, stallTimeoutMs: 1000, totalTimeoutMs: 5000, attempts: 1, env: { NO_PROXY: "127.0.0.1,localhost" }, ...overrides };
}

test("downloads and verifies an asset", async (t) => {
  const body = Buffer.from("verified asset");
  const base = await server(t, (_request, response) => { response.writeHead(200, { "content-length": body.length, etag: '"fixture"' }); response.end(body); });
  const filename = destination(t);
  const result = await controlledDownload({ url: base, size: body.length, sha256: sha(body) }, options(filename));
  assert.equal(fs.readFileSync(filename, "utf8"), body.toString());
  assert.equal(result.sha256, sha(body));
});

test("reuses a complete verified asset without a network request", async (t) => {
  const body = Buffer.from("cached asset");
  const filename = destination(t);
  fs.writeFileSync(filename, body);
  const result = await controlledDownload(
    { url: "https://example.invalid/unused", size: body.length, sha256: sha(body) },
    options(filename),
  );
  assert.equal(result.cache_hit, true);
  assert.equal(result.resumed_from, body.length);
});

test("resumes a matching partial with Range", async (t) => {
  const body = Buffer.from("0123456789abcdef");
  let range;
  const base = await server(t, (request, response) => {
    range = request.headers.range;
    const offset = Number(range?.match(/bytes=(\d+)-/)?.[1] || 0);
    response.writeHead(offset ? 206 : 200, { "content-length": body.length - offset, "content-range": `bytes ${offset}-${body.length - 1}/${body.length}`, etag: '"range"' });
    response.end(body.subarray(offset));
  });
  const filename = destination(t);
  fs.writeFileSync(`${filename}.part`, body.subarray(0, 5));
  fs.writeFileSync(`${filename}.part.json`, JSON.stringify({ identity: "fixture", size: body.length, sha256: sha(body), etag: '"range"' }));
  await controlledDownload({ url: base, size: body.length, sha256: sha(body) }, options(filename));
  assert.equal(range, "bytes=5-");
  assert.deepEqual(fs.readFileSync(filename), body);
});

test("restarts safely when Range is ignored", async (t) => {
  const body = Buffer.from("complete-body");
  const base = await server(t, (_request, response) => { response.writeHead(200, { "content-length": body.length }); response.end(body); });
  const filename = destination(t);
  fs.writeFileSync(`${filename}.part`, body.subarray(0, 4));
  fs.writeFileSync(`${filename}.part.json`, JSON.stringify({ identity: "fixture", size: body.length, sha256: sha(body) }));
  await controlledDownload({ url: base, size: body.length, sha256: sha(body) }, options(filename));
  assert.deepEqual(fs.readFileSync(filename), body);
});

test("classifies HTTP and checksum failures", async (t) => {
  const missing = await server(t, (_request, response) => { response.writeHead(404); response.end(); });
  const first = destination(t);
  await assert.rejects(controlledDownload({ url: missing, size: 1, sha256: sha("x") }, options(first)), (error) => error.code === "DOWNLOAD_HTTP_404");
  const body = Buffer.from("wrong");
  const corrupt = await server(t, (_request, response) => { response.writeHead(200, { "content-length": body.length }); response.end(body); });
  const second = destination(t);
  await assert.rejects(controlledDownload({ url: corrupt, size: body.length, sha256: sha("right") }, options(second)), (error) => error.code === "DOWNLOAD_CHECKSUM_MISMATCH");
});

test("classifies a stalled response", async (t) => {
  const base = await server(t, (_request, response) => { response.writeHead(200, { "content-length": 10 }); response.write("a"); });
  const filename = destination(t);
  await assert.rejects(controlledDownload({ url: base, size: 10, sha256: sha("0123456789") }, options(filename, { stallTimeoutMs: 100, totalTimeoutMs: 1000 })), (error) => error.code === "DOWNLOAD_STALLED");
});

test("classifies an unreachable explicit proxy without exposing credentials", async (t) => {
  const filename = destination(t);
  await assert.rejects(
    controlledDownload(
      { url: "https://example.invalid/asset", size: 1, sha256: sha("x") },
      options(filename, {
        env: { HTTPS_PROXY: "http://account:secret@127.0.0.1:1" },
        connectTimeoutMs: 100,
      }),
    ),
    (error) =>
      error.code === "DOWNLOAD_PROXY_FAILED" &&
      !JSON.stringify(error.details).includes("secret"),
  );
});

test("classifies disk and TLS failures with stable codes", () => {
  assert.equal(
    downloadInternals.classifyNetworkError({ code: "ENOSPC" }, {}).code,
    "DOWNLOAD_DISK_FULL",
  );
  assert.equal(
    downloadInternals.classifyNetworkError(
      { code: "SELF_SIGNED_CERT_IN_CHAIN" },
      {},
    ).code,
    "DOWNLOAD_TLS_FAILED",
  );
});
