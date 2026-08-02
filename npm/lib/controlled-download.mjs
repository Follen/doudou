import crypto from "node:crypto";
import fs from "node:fs";
import path from "node:path";
import { performance } from "node:perf_hooks";

import { Agent, ProxyAgent, fetch } from "undici";

export class DownloadError extends Error {
  constructor(code, message, details = {}, options = {}) {
    super(message, options);
    this.name = "DownloadError";
    this.code = code;
    this.details = details;
    this.retryable = Boolean(options.retryable);
  }
}

function safeUrl(value) {
  const parsed = new URL(value);
  parsed.username = "";
  parsed.password = "";
  parsed.search = "";
  return parsed.toString();
}

function sha256File(filename) {
  const hash = crypto.createHash("sha256");
  const file = fs.openSync(filename, "r");
  const buffer = Buffer.allocUnsafe(1024 * 1024);
  try {
    for (;;) {
      const count = fs.readSync(file, buffer, 0, buffer.length, null);
      if (!count) break;
      hash.update(buffer.subarray(0, count));
    }
  } finally {
    fs.closeSync(file);
  }
  return hash.digest("hex");
}

function atomicJson(filename, value) {
  fs.mkdirSync(path.dirname(filename), { recursive: true });
  const temporary = `${filename}.${process.pid}.tmp`;
  fs.writeFileSync(temporary, `${JSON.stringify(value, null, 2)}\n`, { mode: 0o600 });
  fs.renameSync(temporary, filename);
}

function classifyNetworkError(error, details) {
  if (error instanceof DownloadError) return error;
  const code = error?.cause?.code || error?.code || "";
  if (["ENOTFOUND", "EAI_AGAIN"].includes(code)) {
    return new DownloadError("DOWNLOAD_DNS_FAILED", "The download host could not be resolved.", details, { retryable: true, cause: error });
  }
  if (["CERT_HAS_EXPIRED", "DEPTH_ZERO_SELF_SIGNED_CERT", "SELF_SIGNED_CERT_IN_CHAIN", "UNABLE_TO_VERIFY_LEAF_SIGNATURE"].includes(code)) {
    return new DownloadError("DOWNLOAD_TLS_FAILED", "TLS certificate verification failed.", details, { cause: error });
  }
  if (["UND_ERR_CONNECT_TIMEOUT", "ETIMEDOUT"].includes(code)) {
    return new DownloadError("DOWNLOAD_CONNECT_TIMEOUT", "The download connection timed out.", details, { retryable: true, cause: error });
  }
  if (["ECONNREFUSED", "UND_ERR_SOCKET"].includes(code) && details.proxy) {
    return new DownloadError("DOWNLOAD_PROXY_FAILED", "The configured proxy could not reach the asset.", details, { retryable: true, cause: error });
  }
  if (["ECONNRESET", "ECONNREFUSED", "EPIPE", "UND_ERR_SOCKET"].includes(code)) {
    return new DownloadError("DOWNLOAD_CONNECTION_FAILED", "The asset connection was interrupted.", details, { retryable: true, cause: error });
  }
  if (code === "ENOSPC") {
    return new DownloadError("DOWNLOAD_DISK_FULL", "There is not enough disk space for the asset.", details, { cause: error });
  }
  if (["EACCES", "EPERM", "EROFS"].includes(code)) {
    return new DownloadError("DOWNLOAD_WRITE_FAILED", "The asset directory is not writable.", details, { cause: error });
  }
  return new DownloadError("DOWNLOAD_FAILED", "The asset download failed.", details, { retryable: true, cause: error });
}

function bypassProxy(url, env) {
  const hostname = new URL(url).hostname.toLowerCase();
  const entries = String(env.NO_PROXY || env.no_proxy || "")
    .split(",")
    .map((item) => item.trim().toLowerCase().split(":", 1)[0])
    .filter(Boolean);
  return entries.some((entry) => entry === "*" || hostname === entry || hostname.endsWith(entry.startsWith(".") ? entry : `.${entry}`));
}

function dispatcherFor(url, env, connectTimeoutMs) {
  const proxy = env.HTTPS_PROXY || env.https_proxy || env.HTTP_PROXY || env.http_proxy;
  if (proxy && !bypassProxy(url, env)) {
    return {
      dispatcher: new ProxyAgent({ uri: proxy, connect: { timeout: connectTimeoutMs } }),
      proxy: true,
    };
  }
  return { dispatcher: new Agent({ connect: { timeout: connectTimeoutMs } }), proxy: false };
}

function humanBytes(value) {
  if (!Number.isFinite(value)) return "?";
  const units = ["B", "MiB", "GiB"];
  let count = value;
  let unit = 0;
  while (count >= 1024 && unit < units.length - 1) {
    count /= 1024;
    unit += 1;
  }
  return `${count.toFixed(unit ? 1 : 0)} ${units[unit]}`;
}

async function attemptDownload(asset, options, attempt) {
  const destination = path.resolve(options.destination);
  if (
    fs.existsSync(destination) &&
    fs.statSync(destination).size === asset.size &&
    sha256File(destination) === asset.sha256
  ) {
    options.onProgress?.({
      stage: options.stage,
      attempt,
      downloaded: asset.size,
      total: asset.size,
      speed_bps: 0,
      cache_hit: true,
    });
    return {
      path: destination,
      size: asset.size,
      sha256: asset.sha256,
      resumed_from: asset.size,
      cache_hit: true,
    };
  }
  const partial = `${destination}.part`;
  const metadataFile = `${partial}.json`;
  const expected = { identity: options.identity, size: asset.size, sha256: asset.sha256 };
  let metadata;
  try {
    metadata = JSON.parse(fs.readFileSync(metadataFile, "utf8"));
  } catch {
    metadata = null;
  }
  if (!metadata || metadata.identity !== expected.identity || metadata.size !== expected.size || metadata.sha256 !== expected.sha256) {
    fs.rmSync(partial, { force: true });
    fs.rmSync(metadataFile, { force: true });
    metadata = null;
  }
  let offset = fs.existsSync(partial) ? fs.statSync(partial).size : 0;
  if (offset > asset.size) {
    fs.rmSync(partial, { force: true });
    offset = 0;
  }
  const headers = { "accept-encoding": "identity", "user-agent": "doudou-setup/1" };
  if (offset) {
    headers.range = `bytes=${offset}-`;
    if (metadata?.etag) headers["if-range"] = metadata.etag;
    else if (metadata?.last_modified) headers["if-range"] = metadata.last_modified;
  }
  const controller = new AbortController();
  const started = performance.now();
  let lastByteAt = started;
  let lastHeartbeatAt = started;
  let abortReason = "";
  const totalTimer = setTimeout(() => {
    abortReason = "total";
    controller.abort();
  }, options.totalTimeoutMs);
  const stallTimer = setInterval(() => {
    const now = performance.now();
    if (now - lastHeartbeatAt >= (options.heartbeatMs || 5000)) {
      lastHeartbeatAt = now;
      options.onProgress?.({
        stage: options.stage,
        attempt,
        downloaded: fs.existsSync(partial) ? fs.statSync(partial).size : offset,
        total: asset.size,
        heartbeat: true,
      });
    }
    if (now - lastByteAt > options.stallTimeoutMs) {
      abortReason = "stall";
      controller.abort();
    }
  }, Math.min(1000, Math.max(100, Math.floor(options.stallTimeoutMs / 4))));
  const route = dispatcherFor(asset.url, options.env, options.connectTimeoutMs);
  const details = { asset: options.identity, attempt, proxy: route.proxy, url: safeUrl(asset.url) };
  let response;
  try {
    response = await fetch(asset.url, {
      headers,
      redirect: "follow",
      signal: controller.signal,
      dispatcher: route.dispatcher,
    });
    if (response.status === 416 && offset === asset.size) {
      response.body?.cancel();
    } else if (![200, 206].includes(response.status)) {
      response.body?.cancel();
      const code = response.status === 403 ? "DOWNLOAD_HTTP_403" : response.status === 404 ? "DOWNLOAD_HTTP_404" : "DOWNLOAD_HTTP_ERROR";
      throw new DownloadError(code, `The asset server returned HTTP ${response.status}.`, { ...details, http_status: response.status }, { retryable: response.status >= 500 || response.status === 429 });
    } else {
      let append = offset > 0 && response.status === 206;
      if (append) {
        const contentRange = response.headers.get("content-range") || "";
        append = contentRange.startsWith(`bytes ${offset}-`);
        if (!append) {
          response.body?.cancel();
          throw new DownloadError("DOWNLOAD_RANGE_INVALID", "The server returned an inconsistent byte range.", details, { retryable: true });
        }
      }
      if (offset && response.status === 200) {
        offset = 0;
        append = false;
      }
      fs.mkdirSync(path.dirname(destination), { recursive: true });
      const file = fs.openSync(partial, append ? "a" : "w");
      let downloaded = offset;
      let lastReport = 0;
      const etag = response.headers.get("etag");
      const lastModified = response.headers.get("last-modified");
      atomicJson(metadataFile, { ...expected, etag, last_modified: lastModified, downloaded });
      try {
        for await (const chunk of response.body) {
          fs.writeSync(file, chunk);
          downloaded += chunk.length;
          lastByteAt = performance.now();
          const elapsed = Math.max(0.001, (lastByteAt - started) / 1000);
          if (lastByteAt - lastReport >= 1000) {
            lastReport = lastByteAt;
            atomicJson(metadataFile, { ...expected, etag, last_modified: lastModified, downloaded });
            options.onProgress?.({ stage: options.stage, attempt, downloaded, total: asset.size, speed_bps: Math.max(0, downloaded - offset) / elapsed });
          }
        }
      } finally {
        fs.closeSync(file);
      }
      atomicJson(metadataFile, { ...expected, etag, last_modified: lastModified, downloaded });
    }
  } catch (error) {
    if (abortReason === "stall") {
      throw new DownloadError("DOWNLOAD_STALLED", `No new bytes arrived for ${options.stallTimeoutMs / 1000} seconds.`, details, { retryable: true, cause: error });
    }
    if (abortReason === "total") {
      throw new DownloadError("DOWNLOAD_TOTAL_TIMEOUT", "The download exceeded its total time limit.", details, { retryable: true, cause: error });
    }
    throw classifyNetworkError(error, details);
  } finally {
    clearTimeout(totalTimer);
    clearInterval(stallTimer);
    await route.dispatcher.close();
  }
  const actualSize = fs.statSync(partial).size;
  if (actualSize !== asset.size) {
    throw new DownloadError("DOWNLOAD_SIZE_MISMATCH", `Expected ${humanBytes(asset.size)} but received ${humanBytes(actualSize)}.`, { ...details, expected_size: asset.size, actual_size: actualSize }, { retryable: true });
  }
  const actualHash = sha256File(partial);
  if (actualHash !== asset.sha256) {
    const quarantine = `${partial}.checksum-${Date.now()}`;
    fs.renameSync(partial, quarantine);
    fs.rmSync(metadataFile, { force: true });
    throw new DownloadError("DOWNLOAD_CHECKSUM_MISMATCH", "The downloaded asset SHA-256 did not match the runtime manifest.", { ...details, expected_sha256: asset.sha256, actual_sha256: actualHash, quarantine });
  }
  fs.rmSync(destination, { force: true });
  fs.renameSync(partial, destination);
  fs.rmSync(metadataFile, { force: true });
  options.onProgress?.({ stage: options.stage, attempt, downloaded: actualSize, total: asset.size, speed_bps: 0, completed: true });
  return { path: destination, size: actualSize, sha256: actualHash, resumed_from: offset };
}

export async function controlledDownload(asset, options) {
  const attempts = options.attempts ?? 3;
  const backoff = options.backoffSeconds ?? [1, 3, 10];
  let lastError;
  for (let attempt = 1; attempt <= attempts; attempt += 1) {
    try {
      return await attemptDownload(asset, { ...options, env: options.env || process.env }, attempt);
    } catch (error) {
      lastError = error;
      options.onProgress?.({ stage: options.stage, attempt, error: error.code || "DOWNLOAD_FAILED", retrying: Boolean(error.retryable && attempt < attempts) });
      if (!error.retryable || attempt >= attempts) throw error;
      await new Promise((resolve) => setTimeout(resolve, (backoff[attempt - 1] ?? backoff.at(-1) ?? 1) * 1000));
    }
  }
  throw lastError;
}

export const downloadInternals = { safeUrl, sha256File, classifyNetworkError };
