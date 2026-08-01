"use strict";

const assert = require("node:assert/strict");
const fsp = require("node:fs/promises");
const os = require("node:os");
const path = require("node:path");
const test = require("node:test");
const { buildLaunchOptions, clientHintHeaders } = require("../src/adapter");
const { atomicWriteJson } = require("../src/core");
const {
  createFingerprintRecord,
  fingerprintIntegrity,
  loadFingerprint,
  saveFingerprint,
} = require("../src/fingerprint");
const { createProfile } = require("../src/profile");

async function temporaryDirectory(t) {
  const directory = await fsp.mkdtemp(path.join(os.tmpdir(), "doudou-fingerprint-"));
  t.after(() => fsp.rm(directory, { recursive: true, force: true }));
  return directory;
}

function surfaces() {
  return {
    navigator: {
      user_agent: "Mozilla/5.0 Chrome/140.0.0.0 Safari/537.36",
      app_version: "5.0 Chrome/140.0.0.0 Safari/537.36",
      user_agent_data: {
        brands: [{ brand: "Chromium", version: "140" }, { brand: "Not_A Brand", version: "99" }],
        mobile: false,
        platform: "Windows",
        high_entropy: { architecture: "x86", bitness: "64", platformVersion: "19.0.0" },
      },
      webdriver: false,
      webdriver_descriptor: { enumerable: true, configurable: true },
      platform: "Win32",
      languages: ["zh-CN", "zh"],
      language: "zh-CN",
      hardware_concurrency: 16,
      device_memory: 8,
      timezone: "Asia/Shanghai",
    },
    screen: {
      width: 1920,
      height: 1080,
      avail_width: 1920,
      avail_height: 1040,
      color_depth: 24,
      pixel_depth: 24,
      orientation_type: "landscape-primary",
      orientation_angle: 0,
      device_pixel_ratio: 1,
    },
    canvas: { data_url: "data:image/png;base64,fixture", pixels_sha256: "a".repeat(64), pixels: [4, 5, 6], offscreen_pixels: [1, 2, 3] },
    webgl: {
      webgl: { available: true, vendor: "WebKit", renderer: "WebKit WebGL", version: "WebGL 1.0", shading_language_version: "WebGL GLSL ES 1.0", unmasked_vendor: "Google Inc.", unmasked_renderer: "ANGLE", extensions: [], parameters: {} },
      webgl2: { available: false },
    },
    permissions: { notifications: { state: "prompt", error: null } },
    plugins: [{ name: "PDF Viewer", filename: "internal-pdf-viewer", description: "Portable Document Format", mime_types: ["application/pdf"] }],
    mime_types: [{ type: "application/pdf", suffixes: "pdf", description: "Portable Document Format", enabled_plugin: "PDF Viewer" }],
    media: { source_types: { "video/webm": true }, decoding: {} },
    webrtc: { available: true, video_codecs: [], candidate_policy: "runtime-addresses-not-persisted" },
  };
}

async function persistedFixture(t, profileId = "work") {
  const root = await temporaryDirectory(t);
  const profile = await createProfile(root, profileId);
  const record = createFingerprintRecord({
    profileId: profile.metadata.profile_id,
    browserChannel: "chromium",
    browserVersion: "140.0.0.0",
    surfaces: surfaces(),
  });
  await saveFingerprint(profile.paths.fingerprint, record);
  return { root, profile, record };
}

test("fingerprint identity persists with replayable values across later dates", async (t) => {
  const { profile, record } = await persistedFixture(t);
  const old = {
    ...record,
    captured_at: "2020-01-01T00:00:00.000Z",
    last_verified_at: "2020-01-01T00:00:00.000Z",
  };
  old.integrity_sha256 = fingerprintIntegrity(old);
  await atomicWriteJson(profile.paths.fingerprint, old);
  const loaded = await loadFingerprint(profile.paths.fingerprint, { profileId: profile.metadata.profile_id });
  assert.equal(loaded.captured_at, "2020-01-01T00:00:00.000Z");
  assert.equal(loaded.surfaces.navigator.user_agent, surfaces().navigator.user_agent);
  assert.equal(loaded.surfaces.canvas.data_url, surfaces().canvas.data_url);
});

test("fingerprint integrity and Profile identity fail closed", async (t) => {
  const { profile, record } = await persistedFixture(t);
  await atomicWriteJson(profile.paths.fingerprint, {
    ...record,
    surfaces: { ...record.surfaces, navigator: { ...record.surfaces.navigator, hardware_concurrency: 2 } },
  });
  await assert.rejects(() => loadFingerprint(profile.paths.fingerprint, { profileId: profile.metadata.profile_id }), (error) => {
    assert.equal(error.code, "FINGERPRINT_BASELINE_REQUIRED");
    assert.equal(error.details.reason, "integrity_mismatch");
    return true;
  });
  await saveFingerprint(profile.paths.fingerprint, record);
  await assert.rejects(() => loadFingerprint(profile.paths.fingerprint, { profileId: "another-profile" }), (error) => {
    assert.equal(error.code, "FINGERPRINT_BASELINE_REQUIRED");
    assert.equal(error.details.reason, "profile_mismatch");
    return true;
  });
});

test("missing fingerprint returns explicit and automatic calibration routes", async (t) => {
  const root = await temporaryDirectory(t);
  const profile = await createProfile(root, "work");
  await assert.rejects(() => loadFingerprint(profile.paths.fingerprint, { profileId: "work" }), (error) => {
    assert.equal(error.code, "FINGERPRINT_BASELINE_REQUIRED");
    assert.equal(error.details.reason, "file_missing");
    assert.match(error.details.calibration_command, /profile calibrate --profile work/);
    assert.equal(error.details.auto_fallback_argument, "--auto-calibrate-fallback");
    return true;
  });
});

test("headless launch options replay UA, client hints, screen and automation controls", async (t) => {
  const { record } = await persistedFixture(t);
  const headers = clientHintHeaders(record);
  assert.match(headers["sec-ch-ua"], /Chromium/);
  assert.equal(headers["sec-ch-ua-platform"], '"Windows"');
  const options = buildLaunchOptions({ headless: true, browser: "chromium", fingerprint: record });
  assert.equal(options.userAgent, surfaces().navigator.user_agent);
  assert.deepEqual(options.screen, { width: 1920, height: 1080 });
  assert.ok(options.args.includes("--disable-blink-features=AutomationControlled"));
  assert.deepEqual(options.ignoreDefaultArgs, ["--enable-automation"]);
  assert.equal(options.extraHTTPHeaders["accept-language"], "zh-CN,zh;q=0.9");
});
