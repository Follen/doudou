"use strict";

const path = require("node:path");
const {
  atomicWriteJson,
  atomicWriteJsonl,
  ensureDir,
  now,
  randomId,
  readJson,
  safeSegment,
  sha256,
} = require("./core");
const {
  SELECTORS,
  detectLogin,
  discoverGroups,
  launchSession,
  openChat,
} = require("./adapter");

function jsonPointerEscape(value) {
  return String(value).replaceAll("~", "~0").replaceAll("/", "~1");
}

function valueType(value) {
  if (value === null) return "null";
  if (Array.isArray(value)) return "array";
  if (Number.isInteger(value)) return "integer";
  return typeof value;
}

function inventoryValues(values) {
  const entries = new Map();
  function observe(value, pointer) {
    const type = valueType(value);
    const entry = entries.get(pointer) || { path: pointer || "/", types: new Set(), nullable: false, occurrences: 0, samples: 0 };
    entry.types.add(type);
    entry.nullable ||= value === null;
    entry.occurrences += 1;
    entry.samples = Math.min(entry.samples + 1, 1000);
    entries.set(pointer, entry);
    if (Array.isArray(value)) {
      for (const child of value.slice(0, 1000)) observe(child, `${pointer}/*`);
    } else if (value && typeof value === "object") {
      for (const [key, child] of Object.entries(value)) observe(child, `${pointer}/${jsonPointerEscape(key)}`);
    }
  }
  for (const value of values) observe(value, "");
  return [...entries.values()]
    .map((entry) => ({ ...entry, types: [...entry.types].sort() }))
    .sort((left, right) => left.path.localeCompare(right.path));
}

function fieldDiff(baselineFields, currentFields) {
  const baseline = new Map((baselineFields || []).map((field) => [field.path, field]));
  const current = new Map((currentFields || []).map((field) => [field.path, field]));
  const added = [];
  const removed = [];
  const typeChanged = [];
  const nullabilityChanged = [];
  for (const [fieldPath, field] of current) {
    const previous = baseline.get(fieldPath);
    if (!previous) {
      added.push({ path: fieldPath, current: field });
      continue;
    }
    if (JSON.stringify(previous.types) !== JSON.stringify(field.types)) typeChanged.push({ path: fieldPath, baseline: previous.types, current: field.types });
    if (Boolean(previous.nullable) !== Boolean(field.nullable)) nullabilityChanged.push({ path: fieldPath, baseline: Boolean(previous.nullable), current: Boolean(field.nullable) });
  }
  for (const [fieldPath, field] of baseline) if (!current.has(fieldPath)) removed.push({ path: fieldPath, baseline: field });
  return { added, removed, type_changed: typeChanged, nullability_changed: nullabilityChanged };
}

function lengthBucket(length) {
  if (length === 0) return "0";
  if (length <= 8) return "1-8";
  if (length <= 32) return "9-32";
  if (length <= 128) return "33-128";
  return "129+";
}

function redactShape(value, key = "") {
  if (value === null) return null;
  if (Array.isArray(value)) return value.slice(0, 3).map((item) => redactShape(item, key));
  if (typeof value === "object") {
    const output = {};
    for (const [childKey, child] of Object.entries(value)) {
      if (/cookie|token|authorization|localstorage|sessionstorage|header/i.test(childKey)) {
        output[childKey] = { type: "redacted-secret" };
      } else {
        output[childKey] = redactShape(child, childKey);
      }
    }
    return output;
  }
  if (typeof value === "string") {
    const cleaned = /url|uri|avatar|image/i.test(key) ? value.split(/[?#]/, 1)[0] : value;
    return { type: "string", length: lengthBucket(value.length), sha256: sha256(cleaned) };
  }
  return { type: valueType(value), value: typeof value === "boolean" ? value : undefined };
}

async function inspectDom(page) {
  const contracts = {
    message_entry: SELECTORS.messageEntry,
    conversation_item: SELECTORS.conversationItems,
    message_item: SELECTORS.messageItems,
    composer: SELECTORS.composer,
    send_action: SELECTORS.send,
    reply_action: ['text=回复', '[data-e2e*="reply"]'],
    conversation_title: ['[data-conversation-title]', '[data-e2e*="conversation-title"]', 'h1', 'h2'],
    history_boundary: ['[data-has-more]', '[data-e2e*="history"]', '[class*="message-list"]'],
  };
  const result = [];
  for (const [name, selectors] of Object.entries(contracts)) {
    const candidates = [];
    for (const selector of selectors) {
      const locator = page.locator(selector);
      const count = await locator.count().catch(() => 0);
      let visible = 0;
      for (let index = 0; index < Math.min(count, 20); index += 1) if (await locator.nth(index).isVisible().catch(() => false)) visible += 1;
      candidates.push({ selector, count, visible });
    }
    const matched = candidates.reduce((sum, candidate) => sum + candidate.count, 0);
    const visible = candidates.reduce((sum, candidate) => sum + candidate.visible, 0);
    result.push({ name, candidates, matched, visible, status: matched === 0 ? "missing" : name.includes("title") && visible !== 1 ? "ambiguous" : "present" });
  }
  return result;
}

function domDiff(baselineDom, currentDom) {
  const baseline = new Map((baselineDom || []).map((item) => [item.name, item]));
  const findings = [];
  for (const item of currentDom) {
    const previous = baseline.get(item.name);
    if (item.status === "missing" && previous?.status === "present") findings.push({ code: "DOM_CONTRACT_MISSING", severity: "error", contract_area: "dom", path: item.name, change_type: "removed", summary: "A previously present DOM contract is missing." });
    else if (item.status === "ambiguous") findings.push({ code: "DOM_CONTRACT_AMBIGUOUS", severity: "error", contract_area: "dom", path: item.name, change_type: "ambiguous", summary: "A DOM identity contract is not unique." });
  }
  return findings;
}

async function runPageDoctor({ projectRoot, profileId, browserData, fingerprint, fingerprintPath, baselinePath = null, browser = "chromium" }) {
  const runId = randomId("doctor");
  const startedAt = now();
  const directory = path.join(projectRoot, "message", "profiles", safeSegment(profileId), "diagnostics", "page-doctor", now().slice(0, 10), runId);
  await ensureDir(directory);
  const artifacts = {
    doctor: path.join(directory, "doctor.json"),
    bundle_manifest: path.join(directory, "bundle-manifest.json"),
    field_inventory: path.join(directory, "field-inventory.json"),
    dom_contract: path.join(directory, "dom-contract.json"),
    redacted_samples: path.join(directory, "redacted-samples.jsonl"),
  };
  const session = await launchSession({ browserData, headless: true, browser, fingerprint, fingerprintPath, captureScripts: true });
  let login;
  let groups = [];
  let dom = [];
  try {
    await openChat(session);
    login = await detectLogin(session);
    if (login.valid) {
      groups = await discoverGroups(session);
      dom = await inspectDom(session.page);
    }
    await session.capture.settle();
  } finally {
    await session.context.close();
  }

  const scriptsByPath = new Map();
  for (const script of session.capture.scripts) scriptsByPath.set(script.url, script);
  const scripts = [...scriptsByPath.values()]
    .map((script) => ({ resource_path: script.url, sha256: sha256(script.body), bytes: script.bytes, discovered_at: startedAt, status: "captured" }))
    .sort((left, right) => left.resource_path.localeCompare(right.resource_path));
  const bundleFingerprint = sha256(scripts.map((script) => `${script.resource_path}\0${script.sha256}`).join("\n"));
  const fields = inventoryValues(session.capture.json.map((response) => response.value));
  const baseline = baselinePath ? await readJson(path.resolve(baselinePath), { optional: true }) : null;
  const fieldsDiff = baseline ? fieldDiff(baseline.fields, fields) : { added: [], removed: [], type_changed: [], nullability_changed: [] };
  const findings = [];
  if (!baseline) findings.push({ code: "PAGE_CONTRACT_BASELINE_MISSING", severity: "warning", contract_area: "baseline", path: null, change_type: "missing", summary: "No approved page-contract baseline was supplied." });
  if (baseline?.bundle_fingerprint && baseline.bundle_fingerprint !== bundleFingerprint) findings.push({ code: "BUNDLE_DRIFT", severity: "warning", contract_area: "bundle", path: null, change_type: "changed", summary: "The deterministic bundle content fingerprint changed." });
  if (baseline) {
    for (const item of fieldsDiff.added) findings.push({ code: "FIELD_ADDED", severity: "info", contract_area: "fields", path: item.path, change_type: "added", summary: "A structured field was added." });
    for (const item of fieldsDiff.removed) findings.push({ code: "FIELD_REMOVED", severity: "error", contract_area: "fields", path: item.path, change_type: "removed", summary: "A structured field was removed." });
    for (const item of fieldsDiff.type_changed) findings.push({ code: "FIELD_TYPE_CHANGED", severity: "error", contract_area: "fields", path: item.path, change_type: "type_changed", summary: "A structured field changed type." });
    for (const item of fieldsDiff.nullability_changed) findings.push({ code: "FIELD_NULLABILITY_CHANGED", severity: "warning", contract_area: "fields", path: item.path, change_type: "nullability_changed", summary: "A structured field changed nullability." });
  }
  findings.push(...domDiff(baseline?.dom, dom));
  if (!login.valid) findings.push({ code: login.reason_code || "LOGIN_REQUIRED", severity: "error", contract_area: "auth", path: null, change_type: "unavailable", summary: "The selected Profile requires headed login or verification." });

  const bundleManifest = { schema: "doudou.bundle-manifest.v1", doctor_run_id: runId, bundle_fingerprint: bundleFingerprint, resources: scripts };
  const fieldInventory = { schema: "doudou.field-inventory.v1", doctor_run_id: runId, fields, diff: fieldsDiff };
  const domContract = { schema: "doudou.dom-contract.v1", doctor_run_id: runId, contracts: dom };
  const redactedSamples = session.capture.json.slice(0, 100).map((response, index) => ({ schema: "doudou.redacted-sample.v1", sample_id: `${runId}-${index + 1}`, source_path: response.url, shape: redactShape(response.value) }));
  await Promise.all([
    atomicWriteJson(artifacts.bundle_manifest, bundleManifest),
    atomicWriteJson(artifacts.field_inventory, fieldInventory),
    atomicWriteJson(artifacts.dom_contract, domContract),
    atomicWriteJsonl(artifacts.redacted_samples, redactedSamples),
  ]);
  const hasError = findings.some((finding) => finding.severity === "error");
  const hasDrift = findings.some((finding) => finding.code !== "PAGE_CONTRACT_BASELINE_MISSING" && finding.code !== "LOGIN_REQUIRED");
  const status = !login.valid ? "incomplete" : !baseline ? "incomplete" : hasError || hasDrift ? "drift_detected" : "compatible";
  const relativeArtifacts = Object.fromEntries(Object.entries(artifacts).map(([key, filename]) => [key, path.relative(projectRoot, filename).replaceAll("\\", "/")]));
  const doctor = {
    schema: "doudou.page-doctor.v1",
    doctor_run_id: runId,
    profile_id: profileId,
    started_at: startedAt,
    completed_at: now(),
    status,
    baseline_version: baseline?.version || null,
    bundle_fingerprint: bundleFingerprint,
    groups_observed: groups.length,
    artifacts: relativeArtifacts,
    findings,
  };
  await atomicWriteJson(artifacts.doctor, doctor);
  return { doctor, directory, bundleManifest, fieldInventory, domContract };
}

module.exports = {
  domDiff,
  fieldDiff,
  inventoryValues,
  redactShape,
  runPageDoctor,
};
