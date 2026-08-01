"use strict";

const crypto = require("node:crypto");
const fsp = require("node:fs/promises");
const path = require("node:path");
const { AppError, ensureDir, sha256 } = require("./core");
const { ensureConversation } = require("./storage");

function digest(algorithm, value) {
  return crypto.createHash(algorithm).update(value).digest("hex");
}

function detectImage(buffer) {
  if (buffer.subarray(0, 8).toString("hex") === "89504e470d0a1a0a" && buffer.length >= 24) {
    return { format: "png", extension: "png", mime: "image/png", width: buffer.readUInt32BE(16), height: buffer.readUInt32BE(20) };
  }
  if (buffer.subarray(0, 3).toString("hex") === "ffd8ff") {
    let offset = 2;
    while (offset + 9 < buffer.length) {
      if (buffer[offset] !== 0xff) { offset += 1; continue; }
      const marker = buffer[offset + 1];
      if ([0xc0, 0xc1, 0xc2, 0xc3, 0xc5, 0xc6, 0xc7, 0xc9, 0xca, 0xcb, 0xcd, 0xce, 0xcf].includes(marker)) {
        return { format: "jpeg", extension: "jpg", mime: "image/jpeg", width: buffer.readUInt16BE(offset + 7), height: buffer.readUInt16BE(offset + 5) };
      }
      if (marker === 0xd8 || marker === 0xd9) { offset += 2; continue; }
      const length = buffer.readUInt16BE(offset + 2);
      if (length < 2) break;
      offset += 2 + length;
    }
    throw new AppError("IMAGE_DIMENSIONS_UNRESOLVED", "JPEG dimensions were not found.");
  }
  if (buffer.subarray(0, 4).toString("ascii") === "RIFF" && buffer.subarray(8, 12).toString("ascii") === "WEBP") {
    const chunk = buffer.subarray(12, 16).toString("ascii");
    if (chunk === "VP8X" && buffer.length >= 30) {
      return { format: "webp", extension: "webp", mime: "image/webp", width: 1 + buffer.readUIntLE(24, 3), height: 1 + buffer.readUIntLE(27, 3) };
    }
    if (chunk === "VP8L" && buffer.length >= 25) {
      const bits = buffer.readUInt32LE(21);
      return { format: "webp", extension: "webp", mime: "image/webp", width: (bits & 0x3fff) + 1, height: ((bits >> 14) & 0x3fff) + 1 };
    }
    if (chunk === "VP8 " && buffer.length >= 30) {
      return { format: "webp", extension: "webp", mime: "image/webp", width: buffer.readUInt16LE(26) & 0x3fff, height: buffer.readUInt16LE(28) & 0x3fff };
    }
    throw new AppError("IMAGE_DIMENSIONS_UNRESOLVED", "WebP dimensions were not found.");
  }
  throw new AppError("IMAGE_FORMAT_UNSUPPORTED", "Image must be PNG, JPEG or WebP.");
}

function crc32(buffer) {
  let crc = 0xffffffff;
  for (const byte of buffer) {
    crc ^= byte;
    for (let bit = 0; bit < 8; bit += 1) crc = (crc >>> 1) ^ (0xedb88320 & -(crc & 1));
  }
  return ((crc ^ 0xffffffff) >>> 0).toString(16).padStart(8, "0");
}

function hmac(key, value, encoding = undefined) {
  return crypto.createHmac("sha256", key).update(value).digest(encoding);
}

function rfc3986(value) {
  return encodeURIComponent(String(value)).replace(/[!'()*]/g, (character) => `%${character.charCodeAt(0).toString(16).toUpperCase()}`);
}

function signedRequest({ method, host, path: requestPath = "/", query = {}, headers = {}, body = null, accessKey, secretKey, sessionToken, region = "cn-north-1", service = "vod", date = new Date() }) {
  const amzDate = date.toISOString().replace(/[-:]/g, "").replace(/\.\d{3}Z$/, "Z");
  const shortDate = amzDate.slice(0, 8);
  const payloadHash = sha256(body || Buffer.alloc(0));
  const allHeaders = { "x-amz-date": amzDate, "x-amz-security-token": sessionToken, ...headers };
  if (body !== null) allHeaders["x-amz-content-sha256"] = payloadHash;
  const normalized = Object.fromEntries(Object.entries(allHeaders)
    .filter(([, value]) => value !== undefined && value !== null)
    .map(([key, value]) => [key.toLowerCase(), String(value).trim().replace(/\s+/g, " ")]));
  const unsigned = new Set(["authorization", "content-type", "content-length", "user-agent", "presigned-expires", "expect", "x-amzn-trace-id"]);
  const signedHeaders = Object.keys(normalized).filter((key) => key.startsWith("x-amz-") || !unsigned.has(key)).sort();
  const canonicalQuery = Object.keys(query).sort().map((key) => `${rfc3986(key)}=${rfc3986(query[key])}`).join("&");
  const canonicalHeaders = `${signedHeaders.map((key) => `${key}:${normalized[key]}`).join("\n")}\n`;
  const canonicalRequest = [method.toUpperCase(), requestPath, canonicalQuery, canonicalHeaders, signedHeaders.join(";"), payloadHash].join("\n");
  const scope = `${shortDate}/${region}/${service}/aws4_request`;
  const stringToSign = `AWS4-HMAC-SHA256\n${amzDate}\n${scope}\n${sha256(canonicalRequest)}`;
  const signingKey = hmac(hmac(hmac(hmac(`AWS4${secretKey}`, shortDate), region), service), "aws4_request");
  const signature = hmac(signingKey, stringToSign, "hex");
  return {
    url: `https://${host}${requestPath}${canonicalQuery ? `?${canonicalQuery}` : ""}`,
    headers: { ...normalized, authorization: `AWS4-HMAC-SHA256 Credential=${accessKey}/${scope}, SignedHeaders=${signedHeaders.join(";")}, Signature=${signature}` },
    canonicalRequest,
  };
}

async function jsonFetch(url, options = {}) {
  const response = await fetch(url, options);
  const text = await response.text();
  let body;
  try { body = JSON.parse(text); } catch { body = { raw: text }; }
  if (!response.ok) throw new AppError("IMAGE_UPLOAD_HTTP_FAILED", `Image upload request failed with HTTP ${response.status}.`, { http_status: response.status });
  return { response, body };
}

async function uploadImage(session, filename) {
  const image = await fsp.readFile(filename);
  const detected = detectImage(image);
  const sourceMd5 = digest("md5", image);
  const configResult = await session.page.evaluate(async () => {
    const response = await fetch("/aweme/v1/web/im/upload/config/v2?aid=6383", { credentials: "include" });
    return { status: response.status, text: await response.text() };
  });
  if (configResult.status !== 200) throw new AppError("IMAGE_UPLOAD_CONFIG_FAILED", "Image upload configuration request failed.", { http_status: configResult.status });
  const config = JSON.parse(configResult.text).public_image_config;
  if (!config?.access_key_id || !config?.secret_access_key || !config?.session_token || !config?.space_name) {
    throw new AppError("IMAGE_UPLOAD_CONFIG_INVALID", "public_image_config is incomplete.");
  }
  const apply = signedRequest({
    method: "GET",
    host: "vod.bytedanceapi.com",
    query: {
      Action: "ApplyUploadInner",
      FileSize: image.length,
      FileType: "image",
      IsInner: 1,
      NeedFallback: true,
      SpaceName: config.space_name,
      Version: "2020-11-19",
      s: crypto.randomBytes(8).toString("hex"),
    },
    accessKey: config.access_key_id,
    secretKey: config.secret_access_key,
    sessionToken: config.session_token,
  });
  const applied = await jsonFetch(apply.url, { headers: apply.headers });
  const address = applied.body?.Result?.UploadAddress || applied.body?.Result?.InnerUploadAddress?.UploadNodes?.[0];
  const store = address?.StoreInfos?.[0];
  const uploadHost = address?.UploadHosts?.[0] || address?.UploadHost;
  if (!store?.StoreUri || !store.Auth || !uploadHost) throw new AppError("IMAGE_APPLY_INVALID", "ApplyUploadInner did not return an upload address.");
  const tosResponse = await fetch(`https://${uploadHost}/upload/v1/${store.StoreUri}`, {
    method: "POST",
    headers: {
      authorization: store.Auth,
      "content-type": detected.mime,
      "content-crc32": crc32(image),
      "content-disposition": `attachment; filename="${path.basename(filename).replaceAll('"', "")}"`,
      "x-storage-u": store.StorageHeader?.USER_ID || "",
    },
    body: image,
  });
  const tosText = await tosResponse.text();
  let tosBody;
  try { tosBody = JSON.parse(tosText); } catch { tosBody = null; }
  if (!tosResponse.ok || tosBody?.code !== 2000) throw new AppError("IMAGE_TOS_UPLOAD_FAILED", "TOS image upload failed.", { http_status: tosResponse.status, service_code: tosBody?.code ?? null });
  const sessionKey = applied.body.Result.SessionKey || address.SessionKey;
  const commitBody = Buffer.from(JSON.stringify({
    SessionKey: sessionKey,
    Functions: [{ name: "Encryption", input: { Config: { copies: "cipher_v2" }, PolicyParams: { "policy-set": "check,thumb,medium,large" } } }],
  }));
  const commit = signedRequest({
    method: "POST",
    host: "vod.bytedanceapi.com",
    query: { Action: "CommitUploadInner", SpaceName: config.space_name, Version: "2020-11-19" },
    headers: { "content-type": "application/json" },
    body: commitBody,
    accessKey: config.access_key_id,
    secretKey: config.secret_access_key,
    sessionToken: config.session_token,
  });
  const committed = await jsonFetch(commit.url, { method: "POST", headers: commit.headers, body: commitBody });
  const encryption = committed.body?.Result?.Results?.[0]?.Encryption;
  if (!encryption?.Uri || !encryption.SecretKey) throw new AppError("IMAGE_COMMIT_INVALID", "CommitUploadInner did not return encrypted image metadata.");
  const buildBody = JSON.stringify({ convert_params: [{ uri: encryption.Uri, format: "tplv-x-get:large.image", tpl: "%s://%v/%v~%v" }] });
  const builtResult = await session.page.evaluate(async ({ body }) => {
    const response = await fetch("/aweme/v1/web/privacy/batch_build_image/", { method: "POST", credentials: "include", headers: { "content-type": "application/json" }, body });
    return { status: response.status, text: await response.text() };
  }, { body: buildBody });
  const built = JSON.parse(builtResult.text);
  const urls = built?.data?.pack_results?.[0]?.UrlList || [];
  if (builtResult.status !== 200 || !urls.length) throw new AppError("IMAGE_BUILD_FAILED", "batch_build_image did not return image URLs.", { http_status: builtResult.status });
  return {
    content: {
      resource_url: {
        oid: encryption.Uri,
        skey: encryption.SecretKey,
        data_size: image.length,
        md5: sourceMd5,
        large_url_list: urls,
        medium_url_list: urls,
        origin_url_list: urls,
        thumb_url_list: urls,
        long_url_list: [],
      },
      cover_width: detected.width,
      cover_height: detected.height,
      md5: sourceMd5,
      from_gallery: 1,
    },
    source: { bytes: image.length, md5: sourceMd5, sha256: sha256(image), ...detected },
    stages: { config: "passed", apply: "passed", tos_upload: "passed", commit: "passed", batch_build_image: "passed", built_url_count: urls.length },
  };
}

function firstResourceUrl(resource) {
  for (const key of ["origin_url_list", "large_url_list", "medium_url_list", "thumb_url_list"]) {
    if (Array.isArray(resource?.[key]) && resource[key].length) return resource[key][0];
  }
  return null;
}

function decryptImage(encrypted, secretKey) {
  const key = Buffer.from(String(secretKey), "hex");
  if (key.length !== 32 || encrypted.length < 29) throw new AppError("MEDIA_ENVELOPE_INVALID", "Encrypted image envelope or key is invalid.");
  const decipher = crypto.createDecipheriv("aes-256-gcm", key, encrypted.subarray(0, 12));
  decipher.setAuthTag(encrypted.subarray(-16));
  try {
    return Buffer.concat([decipher.update(encrypted.subarray(12, -16)), decipher.final()]);
  } catch {
    throw new AppError("MEDIA_GCM_AUTH_FAILED", "Image AES-GCM authentication failed.");
  }
}

async function writeAtomic(filename, buffer) {
  await ensureDir(path.dirname(filename));
  const temporary = `${filename}.${process.pid}.${crypto.randomBytes(6).toString("hex")}.tmp`;
  try {
    await fsp.writeFile(temporary, buffer, { flag: "wx" });
    await fsp.rename(temporary, filename);
  } finally {
    await fsp.rm(temporary, { force: true }).catch(() => {});
  }
}

async function collectMessageMedia(dataRoot, profileId, conversation, messages) {
  const { directory } = await ensureConversation(dataRoot, profileId, conversation);
  for (const message of messages) {
    const resource = message.protocol?.content?.resource_url;
    if (Number(message.protocol?.message_type) !== 27 || !resource) continue;
    const url = firstResourceUrl(resource);
    const sentDate = String(message.sent_at || message.collected_at).slice(0, 10);
    const targetDirectory = path.join(directory, "media", sentDate, String(message.message_id));
    let media;
    try {
      if (!url || !resource.skey) throw new AppError("MEDIA_RESOURCE_MISSING", "Image resource URL or skey is missing.");
      const response = await fetch(url);
      if (!response.ok) throw new AppError("MEDIA_DOWNLOAD_FAILED", "Image download failed.", { http_status: response.status });
      const encrypted = Buffer.from(await response.arrayBuffer());
      const clear = decryptImage(encrypted, resource.skey);
      const detected = detectImage(clear);
      const declaredSize = Number(resource.data_size);
      const declaredMd5 = String(message.protocol.content.md5 || "");
      if (!Number.isFinite(declaredSize) || clear.length !== declaredSize) throw new AppError("MEDIA_SIZE_MISMATCH", "Image size does not match protocol metadata.", { declared: declaredSize, actual: clear.length });
      if (!declaredMd5 || digest("md5", clear) !== declaredMd5) throw new AppError("MEDIA_MD5_MISMATCH", "Image MD5 does not match protocol metadata.");
      const filename = path.join(targetDirectory, `${message.message_id}.${detected.extension}`);
      const clearSha256 = sha256(clear);
      const existing = await fsp.readFile(filename).catch(() => null);
      if (!existing || sha256(existing) !== clearSha256) await writeAtomic(filename, clear);
      media = {
        status: "downloaded",
        retryable: false,
        local_path: path.relative(dataRoot, filename).replaceAll("\\", "/"),
        format: detected.format,
        bytes: clear.length,
        md5: declaredMd5,
        sha256: clearSha256,
        resource_url_sha256: sha256(url),
      };
    } catch (error) {
      media = {
        status: "failed",
        retryable: true,
        error_code: error.code || "MEDIA_PROCESSING_FAILED",
        local_path: null,
        resource_url_sha256: url ? sha256(url) : null,
      };
    }
    message.images = [media];
  }
  return messages;
}

module.exports = {
  collectMessageMedia,
  crc32,
  decryptImage,
  detectImage,
  signedRequest,
  uploadImage,
  writeAtomic,
};
