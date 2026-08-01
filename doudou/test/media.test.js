"use strict";

const assert = require("node:assert/strict");
const crypto = require("node:crypto");
const fsp = require("node:fs/promises");
const os = require("node:os");
const path = require("node:path");
const test = require("node:test");
const { decryptImage, detectImage, signedRequest, writeAtomic } = require("../src/media");

test("image format detection reads PNG, JPEG and WebP dimensions", () => {
  const png = Buffer.alloc(24);
  Buffer.from("89504e470d0a1a0a", "hex").copy(png);
  png.writeUInt32BE(138, 16);
  png.writeUInt32BE(152, 20);
  assert.deepEqual(detectImage(png), { format: "png", extension: "png", mime: "image/png", width: 138, height: 152 });

  const jpeg = Buffer.from("ffd8ffc00011080098008a03011100021100031100", "hex");
  assert.deepEqual(detectImage(jpeg), { format: "jpeg", extension: "jpg", mime: "image/jpeg", width: 138, height: 152 });

  const webp = Buffer.alloc(30);
  webp.write("RIFF", 0);
  webp.write("WEBP", 8);
  webp.write("VP8X", 12);
  webp.writeUIntLE(137, 24, 3);
  webp.writeUIntLE(151, 27, 3);
  assert.deepEqual(detectImage(webp), { format: "webp", extension: "webp", mime: "image/webp", width: 138, height: 152 });
});

test("AES-256-GCM media envelopes authenticate before returning plaintext", () => {
  const key = crypto.randomBytes(32);
  const iv = crypto.randomBytes(12);
  const clear = Buffer.from("image bytes");
  const cipher = crypto.createCipheriv("aes-256-gcm", key, iv);
  const encrypted = Buffer.concat([iv, cipher.update(clear), cipher.final(), cipher.getAuthTag()]);
  assert.deepEqual(decryptImage(encrypted, key.toString("hex")), clear);
  encrypted[encrypted.length - 1] ^= 1;
  assert.throws(() => decryptImage(encrypted, key.toString("hex")), (error) => error.code === "MEDIA_GCM_AUTH_FAILED");
});

test("Volcengine SigV4 construction is deterministic and excludes host", () => {
  const built = signedRequest({
    method: "GET",
    host: "vod.bytedanceapi.com",
    query: { Action: "ApplyUploadInner", FileSize: 1006, Version: "2020-11-19" },
    accessKey: "AKID",
    secretKey: "SECRET",
    sessionToken: "TOKEN",
    date: new Date("2026-08-01T04:00:00.000Z"),
  });
  assert.match(built.headers.authorization, /^AWS4-HMAC-SHA256 Credential=AKID\/20260801\/cn-north-1\/vod\/aws4_request,/);
  assert.match(built.canonicalRequest, /^GET\n\/\nAction=ApplyUploadInner&FileSize=1006&Version=2020-11-19\n/m);
  assert.doesNotMatch(built.canonicalRequest, /^host:/m);
  assert.match(built.headers.authorization, /Signature=de9f9d5e0c3ce129537984d1c3b2c69d6d87791cfc3de313bb8be5d664170f91$/);
});

test("atomic media writes leave only the final file", async (t) => {
  const root = await fsp.mkdtemp(path.join(os.tmpdir(), "doudou-media-"));
  t.after(() => fsp.rm(root, { recursive: true, force: true }));
  const filename = path.join(root, "media", "image.png");
  await writeAtomic(filename, Buffer.from("complete"));
  assert.equal(await fsp.readFile(filename, "utf8"), "complete");
  assert.deepEqual((await fsp.readdir(path.dirname(filename))).sort(), ["image.png"]);
});
