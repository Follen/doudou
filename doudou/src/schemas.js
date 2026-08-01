"use strict";

const { AppError } = require("./core");

const requiredFields = {
  "doudou.profile.v1": ["profile_id", "display_name", "browser_data_dir", "created_at", "updated_at", "login_status"],
  "doudou.conversations-index.v1": ["profile_id", "refreshed_at", "count", "conversations"],
  "doudou.conversation.v1": ["profile_id", "conversation_id", "conversation_type", "kind", "display_name", "folder_name", "discovered_at", "updated_at", "message_count"],
  "doudou.message.v1": ["profile_id", "conversation_id", "conversation_name", "message_id", "sender", "type", "images", "collected_at", "run_id", "source", "parse_warnings"],
  "doudou.send-event.v1": ["event_id", "occurred_at", "profile_id", "conversation_id", "event_type", "text", "result"],
  "doudou.groups-index.v1": ["profile_id", "refreshed_at", "count", "groups"],
  "doudou.group.v1": ["profile_id", "conversation_id", "display_name", "folder_name", "discovered_at", "updated_at", "message_count"],
  "doudou.reply-event.v1": ["event_id", "plan_id", "task_id", "occurred_at", "profile_id", "conversation_id", "target_message_id", "event_type", "reply_text", "attempt", "result"],
  "doudou.collection-run.v1": ["run_id", "profile_id", "requested_at", "started_at", "selected_groups", "status", "totals", "groups"],
  "doudou.conflict.v1": ["conflict_id", "occurred_at", "run_id", "conversation_id", "field", "reason_code"],
  "doudou.doctor.v1": ["status", "node", "npm", "playwright", "chromium", "filesystem", "issues"],
  "doudou.page-doctor.v1": ["doctor_run_id", "profile_id", "started_at", "completed_at", "status", "baseline_version", "bundle_fingerprint", "artifacts", "findings"],
  "doudou.fingerprint-profile.v1": ["profile_id", "browser", "captured_at", "last_verified_at", "status", "surfaces", "integrity_sha256"],
};

function validateRecord(value, location = {}) {
  if (!value || typeof value !== "object" || Array.isArray(value)) {
    throw new AppError("SCHEMA_VALIDATION_FAILED", "Record must be a JSON object.", location);
  }
  if (typeof value.schema !== "string" || !requiredFields[value.schema]) {
    throw new AppError("UNKNOWN_SCHEMA", `Unsupported schema: ${String(value.schema)}.`, { ...location, schema: value.schema ?? null });
  }
  const missing = requiredFields[value.schema].filter((field) => !(field in value));
  if (missing.length) {
    throw new AppError("SCHEMA_VALIDATION_FAILED", `Missing required fields for ${value.schema}.`, { ...location, missing });
  }
  if (value.schema === "doudou.groups-index.v1" && value.count !== value.groups.length) {
    throw new AppError("SCHEMA_VALIDATION_FAILED", "groups-index count must equal groups.length.", location);
  }
  if (value.schema === "doudou.conversations-index.v1" && value.count !== value.conversations.length) {
    throw new AppError("SCHEMA_VALIDATION_FAILED", "conversations-index count must equal conversations.length.", location);
  }
  if (value.schema === "doudou.message.v1" && !value.message_id) {
    throw new AppError("SCHEMA_VALIDATION_FAILED", "message_id must be non-empty.", location);
  }
  if (value.schema === "doudou.fingerprint-profile.v1" && value.status !== "valid") {
    throw new AppError("SCHEMA_VALIDATION_FAILED", "fingerprint-profile status must be valid.", location);
  }
  return value;
}

module.exports = { requiredFields, validateRecord };
