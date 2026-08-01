"use strict";

const path = require("node:path");
const { AppError, atomicWriteJson, now, readJson, sha256 } = require("./core");

const FINGERPRINT_SCHEMA = "doudou.fingerprint-profile.v1";
const PERMISSION_NAMES = [
  "geolocation",
  "notifications",
  "camera",
  "microphone",
  "clipboard-read",
  "clipboard-write",
];
const MEDIA_TYPES = [
  'video/mp4; codecs="avc1.42E01E"',
  'video/webm; codecs="vp9"',
  'audio/mp4; codecs="mp4a.40.2"',
  'audio/webm; codecs="opus"',
];

function canonicalize(value) {
  if (Array.isArray(value)) return value.map(canonicalize);
  if (!value || typeof value !== "object") return value;
  return Object.fromEntries(Object.keys(value).sort().map((key) => [key, canonicalize(value[key])]));
}

function fingerprintIntegrity(record) {
  const { integrity_sha256: ignored, ...payload } = record;
  return sha256(JSON.stringify(canonicalize(payload)));
}

function executableRevision(executablePath) {
  const normalized = String(executablePath || "").replaceAll("\\", "/");
  return normalized.match(/(?:chromium|chrome-headless-shell)-([0-9]+)/i)?.[1] || null;
}

function runtimeDescriptor(browserChannel = "chromium") {
  const { chromium } = require("playwright");
  const playwrightVersion = require("playwright/package.json").version;
  const executablePath = browserChannel === "chromium" ? chromium.executablePath() : null;
  return {
    browser_channel: browserChannel,
    playwright_version: playwrightVersion,
    executable_path: executablePath,
    chromium_revision: executableRevision(executablePath),
  };
}

function baselineRequired(profileId, filename, reason, details = {}) {
  return new AppError(
    "FINGERPRINT_BASELINE_REQUIRED",
    `Profile ${profileId} requires headed fingerprint calibration before a headless task can start.`,
    {
      profile_id: profileId,
      fingerprint_file: filename,
      reason,
      calibration_command: `node "${path.join(__dirname, "..", "bin", "doudou.js")}" profile calibrate --profile ${profileId}`,
      auto_fallback_argument: "--auto-calibrate-fallback",
      ...details,
    },
  );
}

function validateFingerprintRecord(record, { profileId, filename }) {
  if (!record || typeof record !== "object" || Array.isArray(record)) {
    throw baselineRequired(profileId, filename, "record_not_object");
  }
  if (record.schema !== FINGERPRINT_SCHEMA) {
    throw baselineRequired(profileId, filename, "schema_incompatible", { actual_schema: record.schema ?? null });
  }
  const required = ["profile_id", "browser", "captured_at", "last_verified_at", "status", "surfaces", "integrity_sha256"];
  const missing = required.filter((field) => !(field in record));
  if (missing.length) throw baselineRequired(profileId, filename, "required_fields_missing", { missing });
  if (record.profile_id !== profileId) {
    throw baselineRequired(profileId, filename, "profile_mismatch", { actual_profile_id: record.profile_id });
  }
  if (record.status !== "valid") throw baselineRequired(profileId, filename, "calibration_not_valid", { status: record.status });
  const expectedIntegrity = fingerprintIntegrity(record);
  if (record.integrity_sha256 !== expectedIntegrity) {
    throw baselineRequired(profileId, filename, "integrity_mismatch", {
      expected_integrity: expectedIntegrity,
      actual_integrity: record.integrity_sha256,
    });
  }
  const surfaceNames = ["navigator", "screen", "canvas", "webgl", "permissions", "plugins", "mime_types", "media", "webrtc"];
  const missingSurfaces = surfaceNames.filter((name) => !record.surfaces[name]);
  if (missingSurfaces.length) throw baselineRequired(profileId, filename, "fingerprint_surfaces_missing", { missing_surfaces: missingSurfaces });
  return record;
}

function assertRuntimeCompatible(record, runtime, filename) {
  const reasons = [];
  if (record.browser.browser_channel !== runtime.browser_channel) reasons.push("browser_channel_changed");
  if (record.browser.playwright_version !== runtime.playwright_version) reasons.push("playwright_version_changed");
  if (record.browser.chromium_revision && runtime.chromium_revision && record.browser.chromium_revision !== runtime.chromium_revision) {
    reasons.push("chromium_revision_changed");
  }
  if (reasons.length) {
    throw baselineRequired(record.profile_id, filename, "browser_contract_incompatible", {
      incompatibilities: reasons,
      baseline_browser: record.browser,
      current_browser: runtime,
    });
  }
}

async function loadFingerprint(filename, { profileId, browserChannel = "chromium" } = {}) {
  let record;
  try {
    record = await readJson(filename, { optional: true });
  } catch (error) {
    if (error.code === "INVALID_JSON") throw baselineRequired(profileId, filename, "invalid_json", { parse_error: error.message });
    throw error;
  }
  if (!record) throw baselineRequired(profileId, filename, "file_missing");
  validateFingerprintRecord(record, { profileId, filename });
  assertRuntimeCompatible(record, runtimeDescriptor(record.browser.browser_channel || browserChannel), filename);
  return record;
}

async function saveFingerprint(filename, record) {
  const complete = { ...record, integrity_sha256: fingerprintIntegrity(record) };
  await atomicWriteJson(filename, complete);
  return complete;
}

async function collectFingerprint(page) {
  return page.evaluate(async ({ permissionNames, mediaTypes }) => {
    const sorted = (values) => [...values].sort((left, right) => String(left).localeCompare(String(right)));
    const bounded = (promise, fallback, milliseconds = 1500) => Promise.race([
      Promise.resolve(promise),
      new Promise((resolve) => setTimeout(() => resolve(fallback), milliseconds)),
    ]);
    const userAgentData = navigator.userAgentData ? {
      brands: navigator.userAgentData.brands.map((item) => ({ brand: item.brand, version: item.version })),
      mobile: navigator.userAgentData.mobile,
      platform: navigator.userAgentData.platform,
      high_entropy: await bounded(navigator.userAgentData.getHighEntropyValues([
        "architecture", "bitness", "formFactors", "fullVersionList", "model", "platformVersion", "uaFullVersion", "wow64",
      ]).catch(() => ({})), {}),
    } : null;

    const canvas = document.createElement("canvas");
    canvas.width = 64;
    canvas.height = 32;
    const twoD = canvas.getContext("2d");
    if (twoD) {
      twoD.fillStyle = "#f60";
      twoD.fillRect(2, 2, 28, 16);
      twoD.font = "11px Arial";
      twoD.fillStyle = "#069";
      twoD.fillText("DGM/v1", 4, 28);
      twoD.globalCompositeOperation = "multiply";
      twoD.fillStyle = "rgba(102, 204, 0, 0.7)";
      twoD.arc(40, 14, 9, 0, Math.PI * 2);
      twoD.fill();
    }
    const canvasSurface = {
      data_url: canvas.toDataURL(),
      pixels_sha256: null,
      pixels: null,
      offscreen_pixels: null,
    };
    if (twoD) {
      const pixels = twoD.getImageData(0, 0, canvas.width, canvas.height).data;
      canvasSurface.pixels = Array.from(pixels);
      if (globalThis.crypto?.subtle) {
        const digest = await crypto.subtle.digest("SHA-256", pixels);
        canvasSurface.pixels_sha256 = [...new Uint8Array(digest)].map((byte) => byte.toString(16).padStart(2, "0")).join("");
      }
    }
    if (typeof OffscreenCanvas === "function") {
      const offscreen = new OffscreenCanvas(16, 16);
      const context = offscreen.getContext("2d");
      context.fillStyle = "#123456";
      context.fillRect(0, 0, 16, 16);
      context.fillStyle = "#abcdef";
      context.fillText("D", 2, 12);
      canvasSurface.offscreen_pixels = Array.from(context.getImageData(0, 0, 16, 16).data);
    }

    const inspectWebGl = (kind) => {
      const probe = document.createElement("canvas").getContext(kind);
      if (!probe) return { available: false };
      const debug = probe.getExtension("WEBGL_debug_renderer_info");
      const parameters = {};
      for (const key of ["MAX_TEXTURE_SIZE", "MAX_CUBE_MAP_TEXTURE_SIZE", "MAX_RENDERBUFFER_SIZE", "MAX_VERTEX_ATTRIBS", "MAX_VIEWPORT_DIMS"]) {
        const enumValue = probe[key];
        const value = probe.getParameter(enumValue);
        parameters[key] = ArrayBuffer.isView(value) ? Array.from(value) : value;
      }
      return {
        available: true,
        vendor: probe.getParameter(probe.VENDOR),
        renderer: probe.getParameter(probe.RENDERER),
        version: probe.getParameter(probe.VERSION),
        shading_language_version: probe.getParameter(probe.SHADING_LANGUAGE_VERSION),
        unmasked_vendor: debug ? probe.getParameter(debug.UNMASKED_VENDOR_WEBGL) : null,
        unmasked_renderer: debug ? probe.getParameter(debug.UNMASKED_RENDERER_WEBGL) : null,
        extensions: sorted(probe.getSupportedExtensions() || []),
        parameters,
      };
    };

    const permissions = {};
    for (const name of permissionNames) {
      try {
        const result = await bounded(navigator.permissions.query({ name }), null);
        if (!result) throw Object.assign(new Error("Permission query timed out."), { name: "TimeoutError" });
        permissions[name] = { state: result.state, error: null };
      } catch (error) {
        permissions[name] = { state: null, error: error?.name || "Error" };
      }
    }

    const plugins = Array.from(navigator.plugins || []).map((plugin) => ({
      name: plugin.name,
      filename: plugin.filename,
      description: plugin.description,
      mime_types: Array.from(plugin).map((mime) => mime.type),
    }));
    const mimeTypes = Array.from(navigator.mimeTypes || []).map((mime) => ({
      type: mime.type,
      suffixes: mime.suffixes,
      description: mime.description,
      enabled_plugin: mime.enabledPlugin?.name || null,
    }));
    const media = { source_types: {}, decoding: {} };
    for (const contentType of mediaTypes) media.source_types[contentType] = globalThis.MediaSource?.isTypeSupported(contentType) || false;
    if (navigator.mediaCapabilities?.decodingInfo) {
      for (const contentType of mediaTypes) {
        const isVideo = contentType.startsWith("video/");
        const config = isVideo
          ? { type: "file", video: { contentType, width: 640, height: 360, bitrate: 500000, framerate: 30 } }
          : { type: "file", audio: { contentType, channels: "2", bitrate: 128000, samplerate: 48000 } };
        try {
          const result = await bounded(navigator.mediaCapabilities.decodingInfo(config), null);
          if (!result) throw Object.assign(new Error("Media capability query timed out."), { name: "TimeoutError" });
          media.decoding[contentType] = { supported: result.supported, smooth: result.smooth, power_efficient: result.powerEfficient };
        } catch (error) {
          media.decoding[contentType] = { error: error?.name || "Error" };
        }
      }
    }

    const capabilities = (globalThis.RTCRtpSender?.getCapabilities?.("video")?.codecs || []).map((codec) => ({
      mime_type: codec.mimeType,
      clock_rate: codec.clockRate,
      channels: codec.channels || null,
      fmtp: codec.sdpFmtpLine || "",
    }));
    return {
      navigator: {
        user_agent: navigator.userAgent,
        app_version: navigator.appVersion,
        user_agent_data: userAgentData,
        webdriver: false,
        webdriver_descriptor: (() => {
          const descriptor = Object.getOwnPropertyDescriptor(Object.getPrototypeOf(navigator), "webdriver");
          return descriptor ? { enumerable: descriptor.enumerable, configurable: descriptor.configurable } : null;
        })(),
        platform: navigator.platform,
        languages: [...navigator.languages],
        language: navigator.language,
        hardware_concurrency: navigator.hardwareConcurrency,
        device_memory: navigator.deviceMemory ?? null,
        timezone: Intl.DateTimeFormat().resolvedOptions().timeZone,
      },
      screen: {
        width: screen.width,
        height: screen.height,
        avail_width: screen.availWidth,
        avail_height: screen.availHeight,
        color_depth: screen.colorDepth,
        pixel_depth: screen.pixelDepth,
        orientation_type: screen.orientation?.type || null,
        orientation_angle: screen.orientation?.angle ?? null,
        device_pixel_ratio: devicePixelRatio,
      },
      canvas: canvasSurface,
      webgl: { webgl: inspectWebGl("webgl"), webgl2: inspectWebGl("webgl2") },
      permissions,
      plugins,
      mime_types: mimeTypes,
      media,
      webrtc: { available: typeof RTCPeerConnection === "function", video_codecs: capabilities, candidate_policy: "runtime-addresses-not-persisted" },
    };
  }, { permissionNames: PERMISSION_NAMES, mediaTypes: MEDIA_TYPES });
}

async function collectFingerprintBounded(page, timeoutMs = 20000) {
  let timer;
  try {
    return await Promise.race([
      collectFingerprint(page),
      new Promise((resolve, reject) => {
        timer = setTimeout(() => reject(new AppError("FINGERPRINT_PROBE_TIMEOUT", "The browser fingerprint probe exceeded its time limit.", { timeout_ms: timeoutMs })), timeoutMs);
      }),
    ]);
  } finally {
    clearTimeout(timer);
  }
}

function normalizeHeadedSurfaces(surfaces) {
  return {
    ...surfaces,
    navigator: { ...surfaces.navigator, webdriver: false },
  };
}

function createFingerprintRecord({ profileId, browserChannel, browserVersion, surfaces }) {
  const runtime = runtimeDescriptor(browserChannel);
  const timestamp = now();
  const record = {
    schema: FINGERPRINT_SCHEMA,
    profile_id: profileId,
    browser: {
      ...runtime,
      product_version: browserVersion || null,
      executable_path: runtime.executable_path ? path.basename(runtime.executable_path) : null,
    },
    captured_at: timestamp,
    last_verified_at: timestamp,
    status: "valid",
    surfaces: normalizeHeadedSurfaces(surfaces),
  };
  return { ...record, integrity_sha256: fingerprintIntegrity(record) };
}

function fingerprintBootstrap(config) {
  const nativeSource = new WeakMap();
  const remember = (replacement, original) => {
    if (typeof replacement === "function" && typeof original === "function") nativeSource.set(replacement, Function.prototype.toString.call(original));
    return replacement;
  };
  const originalToString = Function.prototype.toString;
  const patchedToString = remember(function toString() {
    return nativeSource.get(this) || originalToString.call(this);
  }, originalToString);
  Object.defineProperty(Function.prototype, "toString", { value: patchedToString, configurable: true, writable: true });

  const overrideGetter = (target, name, value) => {
    if (!target) return;
    const descriptor = Object.getOwnPropertyDescriptor(target, name);
    const getter = remember(function get() { return value; }, descriptor?.get || function get() {});
    try { Object.defineProperty(target, name, { get: getter, enumerable: descriptor?.enumerable ?? true, configurable: true }); } catch {}
  };
  const nav = config.navigator;
  const navigatorPrototype = Object.getPrototypeOf(navigator);
  overrideGetter(navigatorPrototype, "userAgent", nav.user_agent);
  overrideGetter(navigatorPrototype, "appVersion", nav.app_version);
  overrideGetter(navigatorPrototype, "platform", nav.platform);
  overrideGetter(navigatorPrototype, "language", nav.language);
  overrideGetter(navigatorPrototype, "languages", Object.freeze([...nav.languages]));
  overrideGetter(navigatorPrototype, "hardwareConcurrency", nav.hardware_concurrency);
  if (nav.device_memory !== null) overrideGetter(navigatorPrototype, "deviceMemory", nav.device_memory);
  overrideGetter(navigatorPrototype, "webdriver", false);

  const makeCollection = (prototype, items, nameKey) => {
    const collection = Object.create(prototype || Array.prototype);
    const values = [...items];
    Object.defineProperty(collection, "length", { value: values.length, enumerable: false });
    values.forEach((item, index) => Object.defineProperty(collection, index, { value: item, enumerable: true }));
    Object.defineProperty(collection, "item", { value: (index) => values[index] || null });
    Object.defineProperty(collection, "namedItem", { value: (name) => values.find((item) => item[nameKey] === name) || null });
    Object.defineProperty(collection, Symbol.iterator, { value: function* iterator() { yield* values; } });
    return collection;
  };
  const pluginPrototype = globalThis.Plugin?.prototype;
  const mimePrototype = globalThis.MimeType?.prototype;
  const plugins = config.plugins.map((item) => {
    const plugin = Object.create(pluginPrototype || Object.prototype);
    Object.defineProperties(plugin, {
      name: { value: item.name, enumerable: true },
      filename: { value: item.filename, enumerable: true },
      description: { value: item.description, enumerable: true },
      length: { value: item.mime_types.length, enumerable: false },
    });
    const mimeValues = item.mime_types.map((type, index) => {
      const mime = Object.create(mimePrototype || Object.prototype);
      Object.defineProperties(mime, {
        type: { value: type, enumerable: true },
        suffixes: { value: type.split("/").pop(), enumerable: true },
        description: { value: item.description, enumerable: true },
        enabledPlugin: { value: plugin, enumerable: true },
      });
      Object.defineProperty(plugin, index, { value: mime, enumerable: true });
      return mime;
    });
    Object.defineProperty(plugin, Symbol.iterator, { value: function* iterator() { yield* mimeValues; } });
    return plugin;
  });
  overrideGetter(navigatorPrototype, "plugins", makeCollection(globalThis.PluginArray?.prototype, plugins, "name"));
  const mimeTypes = config.mime_types.map((item) => {
    const mime = Object.create(mimePrototype || Object.prototype);
    const enabledPlugin = plugins.find((plugin) => plugin.name === item.enabled_plugin) || null;
    Object.defineProperties(mime, {
      type: { value: item.type, enumerable: true },
      suffixes: { value: item.suffixes, enumerable: true },
      description: { value: item.description, enumerable: true },
      enabledPlugin: { value: enabledPlugin, enumerable: true },
    });
    return mime;
  });
  overrideGetter(navigatorPrototype, "mimeTypes", makeCollection(globalThis.MimeTypeArray?.prototype, mimeTypes, "type"));

  if (nav.user_agent_data) {
    const data = nav.user_agent_data;
    const userAgentData = {
      brands: Object.freeze(data.brands.map((item) => Object.freeze({ ...item }))),
      mobile: data.mobile,
      platform: data.platform,
      getHighEntropyValues: async (hints) => {
        const result = { brands: data.brands, mobile: data.mobile, platform: data.platform };
        for (const hint of hints || []) if (hint in data.high_entropy) result[hint] = data.high_entropy[hint];
        return result;
      },
      toJSON() { return { brands: this.brands, mobile: this.mobile, platform: this.platform }; },
    };
    overrideGetter(navigatorPrototype, "userAgentData", userAgentData);
  }

  const screenPrototype = Object.getPrototypeOf(screen);
  for (const [property, key] of Object.entries({ width: "width", height: "height", availWidth: "avail_width", availHeight: "avail_height", colorDepth: "color_depth", pixelDepth: "pixel_depth" })) {
    overrideGetter(screenPrototype, property, config.screen[key]);
  }
  overrideGetter(globalThis, "devicePixelRatio", config.screen.device_pixel_ratio);

  const patchCanvasPixels = (prototype, expectedWidth, expectedHeight, baselinePixels) => {
    if (!prototype || !Array.isArray(baselinePixels)) return;
    const original = prototype.getImageData;
    const replacement = remember(function getImageData(x, y, width, height, ...rest) {
      const image = original.call(this, x, y, width, height, ...rest);
      if (x === 0 && y === 0 && width === expectedWidth && height === expectedHeight
          && this.canvas?.width === expectedWidth && this.canvas?.height === expectedHeight
          && image.data.length === baselinePixels.length) {
        image.data.set(baselinePixels);
      }
      return image;
    }, original);
    try { Object.defineProperty(prototype, "getImageData", { value: replacement, configurable: true, writable: true }); } catch {}
  };
  patchCanvasPixels(globalThis.CanvasRenderingContext2D?.prototype, 64, 32, config.canvas.pixels);
  patchCanvasPixels(globalThis.OffscreenCanvasRenderingContext2D?.prototype, 16, 16, config.canvas.offscreen_pixels);
  if (globalThis.HTMLCanvasElement?.prototype && config.canvas.data_url) {
    const original = HTMLCanvasElement.prototype.toDataURL;
    const replacement = remember(function toDataURL(...args) {
      if (this.width === 64 && this.height === 32 && (!args[0] || args[0] === "image/png")) return config.canvas.data_url;
      return original.apply(this, args);
    }, original);
    try { Object.defineProperty(HTMLCanvasElement.prototype, "toDataURL", { value: replacement, configurable: true, writable: true }); } catch {}
  }

  const patchWebGl = (prototype, baseline) => {
    if (!prototype || !baseline?.available) return;
    const original = prototype.getParameter;
    const replacement = remember(function getParameter(parameter) {
      if (parameter === 0x1F00) return baseline.vendor;
      if (parameter === 0x1F01) return baseline.renderer;
      if (parameter === 0x1F02) return baseline.version;
      if (parameter === 0x8B8C) return baseline.shading_language_version;
      if (parameter === 0x9245) return baseline.unmasked_vendor;
      if (parameter === 0x9246) return baseline.unmasked_renderer;
      return original.call(this, parameter);
    }, original);
    try { Object.defineProperty(prototype, "getParameter", { value: replacement, configurable: true, writable: true }); } catch {}
  };
  patchWebGl(globalThis.WebGLRenderingContext?.prototype, config.webgl.webgl);
  patchWebGl(globalThis.WebGL2RenderingContext?.prototype, config.webgl.webgl2);

  if (navigator.permissions?.query) {
    const originalQuery = navigator.permissions.query.bind(navigator.permissions);
    const replacement = remember(async function query(descriptor) {
      const baseline = config.permissions[descriptor?.name];
      if (!baseline || baseline.error) return originalQuery(descriptor);
      const result = await originalQuery(descriptor).catch(() => null);
      if (result) {
        try { Object.defineProperty(result, "state", { get: () => baseline.state, configurable: true }); } catch {}
        return result;
      }
      const status = new EventTarget();
      Object.defineProperties(status, { state: { get: () => baseline.state }, onchange: { value: null, writable: true } });
      return status;
    }, navigator.permissions.query);
    try { Object.defineProperty(navigator.permissions, "query", { value: replacement, configurable: true }); } catch {}
  }
}

async function applyFingerprint(context, record) {
  await context.addInitScript(fingerprintBootstrap, record.surfaces);
}

function diffValues(expected, actual, prefix = "", differences = []) {
  if (differences.length >= 100) return differences;
  if (Object.is(expected, actual)) return differences;
  if (Array.isArray(expected) || Array.isArray(actual)) {
    if (!Array.isArray(expected) || !Array.isArray(actual) || expected.length !== actual.length) {
      differences.push({ path: prefix || "/", expected, actual });
      return differences;
    }
    for (let index = 0; index < expected.length; index += 1) diffValues(expected[index], actual[index], `${prefix}/${index}`, differences);
    return differences;
  }
  if (expected && actual && typeof expected === "object" && typeof actual === "object") {
    const keys = new Set([...Object.keys(expected), ...Object.keys(actual)]);
    for (const key of [...keys].sort()) diffValues(expected[key], actual[key], `${prefix}/${key}`, differences);
    return differences;
  }
  differences.push({ path: prefix || "/", expected, actual });
  return differences;
}

function comparableSurfaces(surfaces) {
  return canonicalize(surfaces);
}

async function verifyFingerprintPage(page, record) {
  const actual = normalizeHeadedSurfaces(await collectFingerprintBounded(page));
  for (const [contentType, expected] of Object.entries(record.surfaces.media?.decoding || {})) {
    if (actual.media?.decoding?.[contentType]?.error === "TimeoutError") actual.media.decoding[contentType] = structuredClone(expected);
  }
  const differences = diffValues(comparableSurfaces(record.surfaces), comparableSurfaces(actual));
  if (differences.length) {
    throw new AppError("FINGERPRINT_ALIGNMENT_FAILED", "The headless fingerprint does not match the persisted headed baseline.", {
      profile_id: record.profile_id,
      differences: differences.slice(0, 50),
      differences_truncated: differences.length > 50,
      calibration_command: `node "${path.join(__dirname, "..", "bin", "doudou.js")}" profile calibrate --profile ${record.profile_id}`,
    });
  }
  return actual;
}

module.exports = {
  FINGERPRINT_SCHEMA,
  applyFingerprint,
  baselineRequired,
  canonicalize,
  collectFingerprint,
  collectFingerprintBounded,
  createFingerprintRecord,
  diffValues,
  fingerprintBootstrap,
  fingerprintIntegrity,
  loadFingerprint,
  runtimeDescriptor,
  saveFingerprint,
  validateFingerprintRecord,
  verifyFingerprintPage,
};
