"use strict";

const path = require("node:path");
const { AppError, now } = require("./core");
const { detectLogin, discoverConversations, inspectWarmupSurface, launchSession, openChat } = require("./adapter");
const { runDoctor } = require("./doctor");
const { loadFingerprint } = require("./fingerprint");
const { loadProfile, updateProfile, withProfileLock } = require("./profile");
const { readLatestWarmup, writeConversationsIndex, writeWarmupSnapshot } = require("./storage");

const WARMUP_CACHE_MS = 2 * 60 * 60 * 1000;

function relative(projectRoot, filename) {
  return path.relative(projectRoot, filename).replaceAll("\\", "/");
}

function summarizeConversations(index) {
  return {
    count: index.count,
    direct_count: index.conversations.filter((item) => item.kind === "direct").length,
    group_count: index.conversations.filter((item) => item.kind === "group").length,
    active_count: index.conversations.filter((item) => item.discovery_status === "active").length,
    unresolved_count: index.conversations.filter((item) => item.discovery_status !== "active").length,
  };
}

function readFreshWarmup(snapshot, clock = Date.now()) {
  if (snapshot?.status !== "success" || !snapshot.completed_at || !snapshot.conversations_index) return null;
  const ageMs = clock - Date.parse(snapshot.completed_at);
  if (!Number.isFinite(ageMs) || ageMs < 0 || ageMs > WARMUP_CACHE_MS) return null;
  return { snapshot, index: snapshot.conversations_index, age_ms: ageMs };
}

function cachedWarmupResult(projectRoot, profileId, cached, source, liveError = null) {
  return {
    warmup_source: source,
    snapshot: cached.snapshot,
    index: cached.index,
    age_ms: cached.age_ms,
    live_error: liveError,
    files: { latest: relative(projectRoot, path.join(projectRoot, "message", "profiles", profileId, "warmup", "latest.json")) },
  };
}

async function liveWarmup({ projectRoot, profileId, terminateOwner = false }) {
  const doctor = await runDoctor({ projectRoot, launchProbe: true });
  if (doctor.status !== "ok") throw new AppError("DOCTOR_FAILED", "Environment doctor did not pass before warmup.", { status: doctor.status, issues: doctor.issues });
  return withProfileLock(projectRoot, profileId, { terminateOwner }, async () => {
    const startedAt = now();
    const { paths } = await loadProfile(projectRoot, profileId);
    const fingerprint = await loadFingerprint(paths.fingerprint, { profileId });
    const session = await launchSession({
      browserData: paths.browserData,
      headless: true,
      browser: fingerprint.browser.browser_channel,
      fingerprint,
      fingerprintPath: paths.fingerprint,
    });
    try {
      await openChat(session);
      const login = await detectLogin(session);
      if (!login.valid) {
        const code = login.reason_code || "LOGIN_REQUIRED";
        await updateProfile(projectRoot, profileId, { login_status: login.status, last_error_code: code });
        throw new AppError(code, "Profile login is not valid for warmup.", { profile_id: profileId });
      }
      await updateProfile(projectRoot, profileId, { login_status: "valid", last_login_verified_at: now(), last_error_code: null });
      const conversations = await discoverConversations(session);
      const index = await writeConversationsIndex(projectRoot, profileId, conversations);
      const surface = {
        entry: {
          requested_path: "/chat",
          final_path: new URL(session.page.url()).pathname,
        },
        ...await inspectWarmupSurface(session),
      };
      const completedAt = now();
      const snapshot = {
        schema: "doudou.warmup.v1",
        profile_id: profileId,
        started_at: startedAt,
        completed_at: completedAt,
        status: "success",
        headless: true,
        fingerprint_captured_at: fingerprint.captured_at,
        conversations: summarizeConversations(index),
        conversations_index: index,
        contract: surface,
      };
      const files = await writeWarmupSnapshot(projectRoot, profileId, snapshot);
      return {
        warmup_source: "live",
        snapshot,
        index,
        age_ms: 0,
        files: { snapshot: relative(projectRoot, files.filename), latest: relative(projectRoot, files.latest) },
      };
    } finally {
      await session.context.close().catch(() => {});
    }
  });
}

async function runWarmup(options) {
  if (options.preferCache) {
    const cached = readFreshWarmup(await readLatestWarmup(options.projectRoot, options.profileId));
    if (cached) return cachedWarmupResult(options.projectRoot, options.profileId, cached, "fresh_cache");
  }
  try {
    return await liveWarmup(options);
  } catch (error) {
    if (!options.allowStale) throw error;
    const cached = readFreshWarmup(await readLatestWarmup(options.projectRoot, options.profileId));
    if (!cached) {
      throw new AppError("WARMUP_FAILED", "Live warmup failed and no successful snapshot from the last 2 hours is available.", {
        live_error_code: error.code || "WARMUP_LIVE_FAILED",
        live_error_message: error.message,
        cache_max_age_ms: WARMUP_CACHE_MS,
      });
    }
    return cachedWarmupResult(options.projectRoot, options.profileId, cached, "stale_cache", { code: error.code || "WARMUP_LIVE_FAILED", message: error.message });
  }
}

module.exports = { WARMUP_CACHE_MS, liveWarmup, readFreshWarmup, runWarmup, summarizeConversations };
