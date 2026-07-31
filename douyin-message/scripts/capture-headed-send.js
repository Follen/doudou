"use strict";

const fsp = require("node:fs/promises");
const path = require("node:path");
const { now, parseArgs, randomId, safeSegment } = require("../src/core");
const { launchSession, openHome } = require("../src/adapter");
const { loadFingerprint } = require("../src/fingerprint");
const { loadProfile, withProfileLock } = require("../src/profile");

function redactUrl(value) {
  const url = new URL(value);
  for (const key of [...url.searchParams.keys()]) {
    if (/token|signature|bogus|cookie|device|did|verify|ticket/i.test(key)) url.searchParams.set(key, "[REDACTED]");
  }
  return url.toString();
}

function redactHeaders(headers) {
  return Object.fromEntries(Object.entries(headers).map(([key, value]) => [key, /authorization|cookie|token|ticket|signature|x-bogus/i.test(key) ? "[REDACTED]" : value]));
}

function artifactTimestamp(iso) {
  return iso.replace(/[-:]/g, "").replace(".", "");
}

function isCandidate(record) {
  return /(?:im|message|send|conversation|chat)/i.test(record.url)
    || /(?:protobuf|octet-stream)/i.test(record.request_headers["content-type"] || "");
}

async function main() {
  const args = parseArgs(process.argv.slice(2));
  const projectRoot = path.resolve(String(args.data_root || args.project_root || process.env.DOUYIN_MESSAGE_HOME || process.cwd()));
  const profileId = String(args.profile || "").trim();
  if (!profileId) throw new Error("--profile is required");
  await withProfileLock(projectRoot, profileId, { terminateOwner: Boolean(args.terminate_owner) }, async () => {
    const { paths } = await loadProfile(projectRoot, profileId);
    const fingerprint = await loadFingerprint(paths.fingerprint, { profileId });
    const startedAt = now();
    const captureId = randomId("capture");
    const directory = path.join(projectRoot, "message", "profiles", safeSegment(profileId), "diagnostics", "captures", startedAt.slice(0, 10), `${artifactTimestamp(startedAt)}__${safeSegment(captureId)}`);
    const bodiesDirectory = path.join(directory, "bodies");
    await fsp.mkdir(bodiesDirectory, { recursive: true });

    const session = await launchSession({
      browserData: paths.browserData,
      headless: false,
      browser: fingerprint.browser.browser_channel,
      fingerprint,
      fingerprintPath: paths.fingerprint,
    });
    const records = [];
    const byRequest = new WeakMap();
    const pending = new Set();
    const captureErrors = [];
    let sequence = 0;

    function track(promise) {
      const guarded = promise.catch((error) => {
        captureErrors.push({ occurred_at: now(), message: error.message });
      });
      pending.add(guarded);
      guarded.finally(() => pending.delete(guarded));
    }

    try {
      await openHome(session);
      session.context.on("request", (request) => {
        if (request.method() === "GET") return;
        const task = (async () => {
          const id = String(++sequence).padStart(4, "0");
          const requestBody = request.postDataBuffer();
          const requestBodyFile = requestBody ? path.join(bodiesDirectory, `${id}-request.bin`) : null;
          if (requestBodyFile) await fsp.writeFile(requestBodyFile, requestBody);
          const record = {
            id,
            started_at: now(),
            method: request.method(),
            url: redactUrl(request.url()),
            resource_type: request.resourceType(),
            request_headers: redactHeaders(await request.allHeaders().catch(() => request.headers())),
            request_body_file: requestBodyFile ? path.relative(directory, requestBodyFile).replaceAll("\\", "/") : null,
            request_body_bytes: requestBody?.length || 0,
            response_status: null,
            response_headers: null,
            response_body_file: null,
            response_body_bytes: null,
            response_error: null,
          };
          record.candidate = isCandidate(record);
          records.push(record);
          byRequest.set(request, record);
        })();
        track(task);
      });

      session.context.on("response", (response) => {
        if (response.request().method() === "GET") return;
        const task = (async () => {
          await Promise.allSettled([...pending]);
          const record = byRequest.get(response.request());
          if (!record) return;
          record.completed_at = now();
          record.response_status = response.status();
          record.response_headers = redactHeaders(await response.allHeaders().catch(() => response.headers()));
          try {
            const body = await response.body();
            const filename = path.join(bodiesDirectory, `${record.id}-response.bin`);
            await fsp.writeFile(filename, body);
            record.response_body_file = path.relative(directory, filename).replaceAll("\\", "/");
            record.response_body_bytes = body.length;
          } catch (error) {
            record.response_error = error.message;
          }
        })();
        track(task);
      });

      process.stdout.write(`${JSON.stringify({ event: "capture_ready", profile_id: profileId, capture_id: captureId, directory })}\n`);
      await session.page.waitForEvent("close", { timeout: 0 }).catch(() => {});
    } finally {
      while (pending.size) await Promise.allSettled([...pending]);
      await session.context.close().catch(() => {});
      records.sort((left, right) => left.id.localeCompare(right.id));
      const manifest = {
        schema: "douyin-message.headed-send-capture.v1",
        capture_id: captureId,
        profile_id: profileId,
        completed_at: now(),
        request_count: records.length,
        candidate_ids: records.filter((record) => record.candidate).map((record) => record.id),
        capture_errors: captureErrors,
        records,
      };
      const manifestPath = path.join(directory, "manifest.json");
      await fsp.writeFile(manifestPath, `${JSON.stringify(manifest, null, 2)}\n`);
      process.stdout.write(`${JSON.stringify({ event: "capture_completed", capture_id: captureId, manifest: manifestPath, request_count: records.length, candidate_ids: manifest.candidate_ids })}\n`);
    }
  });
}

main().catch((error) => {
  process.stderr.write(`${JSON.stringify({ event: "error", code: error.code || "CAPTURE_FAILED", message: error.message })}\n`);
  process.exitCode = 1;
});
