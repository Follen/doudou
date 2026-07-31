"use strict";

const fs = require("node:fs");
const fsp = require("node:fs/promises");
const path = require("node:path");
const {
  AppError,
  appendJsonl,
  atomicWriteJson,
  atomicWriteJsonl,
  ensureDir,
  now,
  randomId,
  readJson,
  readJsonl,
  safeSegment,
  sha256,
} = require("./core");
const { validateRecord } = require("./schemas");

function dataProfileRoot(projectRoot, profileId) {
  return path.join(projectRoot, "message", "profiles", safeSegment(profileId));
}

async function ensureDataProfile(projectRoot, profileId) {
  const root = dataProfileRoot(projectRoot, profileId);
  await Promise.all([ensureDir(path.join(root, "conversations")), ensureDir(path.join(root, "groups")), ensureDir(path.join(root, "runs"))]);
  return root;
}

function groupFolderName(displayName, conversationId) {
  return `${safeSegment(displayName || "group", 80)}__${safeSegment(conversationId, 80)}`;
}

function conversationFolderName(conversation) {
  const kind = conversation.kind === "direct" || Number(conversation.conversation_type) === 1 ? "direct" : "group";
  return `${kind}__${safeSegment(conversation.display_name || "conversation", 72)}__${safeSegment(conversation.conversation_id, 80)}`;
}

async function writeConversationsIndex(projectRoot, profileId, conversations) {
  const root = await ensureDataProfile(projectRoot, profileId);
  const refreshedAt = now();
  const normalized = conversations.map((conversation) => {
    const conversationType = Number(conversation.conversation_type || (conversation.kind === "direct" ? 1 : 2));
    const kind = conversation.kind || (conversationType === 1 ? "direct" : "group");
    return {
      conversation_id: String(conversation.conversation_id),
      conversation_short_id: String(conversation.conversation_short_id || conversation.conversation_id),
      conversation_type: conversationType,
      kind,
      display_name: String(conversation.display_name || (kind === "direct" ? "未命名私聊" : "未命名群聊")),
      folder_name: conversationFolderName({ ...conversation, kind }),
      last_message_at: conversation.last_message_at || null,
      discovery_status: conversation.discovery_status || "active",
    };
  });
  const index = {
    schema: "douyin-message.conversations-index.v1",
    profile_id: safeSegment(profileId),
    refreshed_at: refreshedAt,
    count: normalized.length,
    conversations: normalized,
  };
  validateRecord(index);
  await atomicWriteJson(path.join(root, "conversations.json"), index);
  return index;
}

async function writeWarmupSnapshot(projectRoot, profileId, snapshot) {
  const root = await ensureDataProfile(projectRoot, profileId);
  const directory = path.join(root, "warmup");
  const dayDirectory = path.join(directory, snapshot.completed_at.slice(0, 10));
  await ensureDir(dayDirectory);
  const stamp = snapshot.completed_at.replace(/[-:.]/g, "");
  const filename = path.join(dayDirectory, `${stamp}__warmup.json`);
  await atomicWriteJson(filename, snapshot);
  await atomicWriteJson(path.join(directory, "latest.json"), snapshot);
  return { directory, filename, latest: path.join(directory, "latest.json") };
}

async function readLatestWarmup(projectRoot, profileId) {
  return readJson(path.join(dataProfileRoot(projectRoot, profileId), "warmup", "latest.json"), { optional: true });
}

async function readConversationsIndex(projectRoot, profileId) {
  const root = dataProfileRoot(projectRoot, profileId);
  const filename = path.join(root, "conversations.json");
  const index = await readJson(filename, { optional: true });
  if (index) {
    validateRecord(index, { path: filename });
    return index;
  }
  const legacy = await readJson(path.join(root, "groups.json"), { optional: true });
  if (!legacy) throw new AppError("CONVERSATION_INDEX_NOT_FOUND", "Refresh conversations before selecting or collecting messages.", { path: filename });
  return {
    schema: "douyin-message.conversations-index.v1",
    profile_id: legacy.profile_id,
    refreshed_at: legacy.refreshed_at,
    count: legacy.groups.length,
    conversations: legacy.groups.map((group) => ({ ...group, conversation_short_id: group.conversation_id, conversation_type: 2, kind: "group" })),
  };
}

async function writeGroupsIndex(projectRoot, profileId, groups) {
  const root = await ensureDataProfile(projectRoot, profileId);
  const refreshedAt = now();
  const normalized = groups.map((group) => ({
    conversation_id: String(group.conversation_id),
    display_name: String(group.display_name || "未命名群聊"),
    folder_name: groupFolderName(group.display_name, group.conversation_id),
    last_message_at: group.last_message_at || null,
    discovery_status: group.discovery_status || "active",
  }));
  const index = {
    schema: "douyin-group-message.groups-index.v1",
    profile_id: safeSegment(profileId),
    refreshed_at: refreshedAt,
    count: normalized.length,
    groups: normalized,
  };
  validateRecord(index);
  await atomicWriteJson(path.join(root, "groups.json"), index);
  return index;
}

async function readGroupsIndex(projectRoot, profileId) {
  const filename = path.join(dataProfileRoot(projectRoot, profileId), "groups.json");
  const index = await readJson(filename, { optional: true });
  if (!index) throw new AppError("GROUP_INDEX_NOT_FOUND", "Refresh groups before using a conversation selection.", { path: filename });
  validateRecord(index, { path: filename });
  return index;
}

async function ensureConversation(projectRoot, profileId, conversation) {
  const root = await ensureDataProfile(projectRoot, profileId);
  const conversationsRoot = path.join(root, "conversations");
  const expectedName = conversationFolderName(conversation);
  const directory = path.join(conversationsRoot, expectedName);
  await ensureDir(directory);
  const filename = path.join(directory, "conversation.json");
  const existing = await readJson(filename, { optional: true });
  const timestamp = now();
  const conversationType = Number(conversation.conversation_type || (conversation.kind === "direct" ? 1 : 2));
  const kind = conversation.kind || (conversationType === 1 ? "direct" : "group");
  const metadata = {
    schema: "douyin-message.conversation.v1",
    profile_id: safeSegment(profileId),
    conversation_id: String(conversation.conversation_id),
    conversation_short_id: String(conversation.conversation_short_id || conversation.conversation_id),
    conversation_type: conversationType,
    kind,
    display_name: String(conversation.display_name || (kind === "direct" ? "未命名私聊" : "未命名群聊")),
    folder_name: expectedName,
    discovered_at: existing?.discovered_at || timestamp,
    updated_at: timestamp,
    last_successful_run_id: existing?.last_successful_run_id || null,
    oldest_collected_at: existing?.oldest_collected_at || null,
    newest_collected_at: existing?.newest_collected_at || null,
    message_count: existing?.message_count || 0,
  };
  validateRecord(metadata);
  await atomicWriteJson(filename, metadata);
  return { directory, metadata };
}

async function ensureGroup(projectRoot, profileId, group) {
  const root = await ensureDataProfile(projectRoot, profileId);
  const groupsRoot = path.join(root, "groups");
  const expectedName = groupFolderName(group.display_name, group.conversation_id);
  let directory = path.join(groupsRoot, expectedName);
  const entries = await fsp.readdir(groupsRoot, { withFileTypes: true });
  for (const entry of entries) {
    if (!entry.isDirectory()) continue;
    const metadataPath = path.join(groupsRoot, entry.name, "group.json");
    const metadata = await readJson(metadataPath, { optional: true });
    if (metadata?.conversation_id !== String(group.conversation_id)) continue;
    if (entry.name !== expectedName) {
      const target = path.join(groupsRoot, expectedName);
      if (!fs.existsSync(target)) await fsp.rename(path.join(groupsRoot, entry.name), target);
      directory = target;
    } else {
      directory = path.join(groupsRoot, entry.name);
    }
    break;
  }
  await ensureDir(directory);
  const filename = path.join(directory, "group.json");
  const existing = await readJson(filename, { optional: true });
  const timestamp = now();
  const metadata = {
    schema: "douyin-group-message.group.v1",
    profile_id: safeSegment(profileId),
    conversation_id: String(group.conversation_id),
    display_name: String(group.display_name || "未命名群聊"),
    folder_name: expectedName,
    discovered_at: existing?.discovered_at || timestamp,
    updated_at: timestamp,
    last_successful_run_id: existing?.last_successful_run_id || null,
    oldest_collected_at: existing?.oldest_collected_at || null,
    newest_collected_at: existing?.newest_collected_at || null,
    message_count: existing?.message_count || 0,
  };
  validateRecord(metadata);
  await atomicWriteJson(filename, metadata);
  return { directory, metadata };
}

function equalValue(left, right) {
  return JSON.stringify(left) === JSON.stringify(right);
}

function mergeValue(existing, incoming, field, conflicts) {
  if ((existing === null || existing === undefined || existing === "") && incoming !== null && incoming !== undefined && incoming !== "") {
    return structuredClone(incoming);
  }
  if (incoming === null || incoming === undefined || incoming === "") return existing;
  if (existing && incoming && typeof existing === "object" && typeof incoming === "object" && !Array.isArray(existing) && !Array.isArray(incoming)) {
    const output = structuredClone(existing);
    for (const [key, value] of Object.entries(incoming)) output[key] = mergeValue(output[key], value, `${field}.${key}`, conflicts);
    return output;
  }
  if (!equalValue(existing, incoming)) conflicts.push({ field, existing_value: existing, incoming_value: incoming });
  return existing;
}

function mergeMessage(existing, incoming, conflicts) {
  const merged = structuredClone(existing);
  for (const [field, incomingValue] of Object.entries(incoming)) {
    if (["schema", "message_id", "collected_at", "run_id"].includes(field)) continue;
    if (field === "type" && ((incoming.protocol && merged.type !== incomingValue) || (merged.type === "unsupported" && incomingValue !== "unsupported"))) {
      merged.type = incomingValue;
      continue;
    }
    if (field === "images" && Array.isArray(merged.images) && merged.images.length === 0 && Array.isArray(incomingValue) && incomingValue.length) {
      merged.images = structuredClone(incomingValue);
      continue;
    }
    if (field === "protocol" && incomingValue && typeof incomingValue === "object") {
      merged.protocol = structuredClone(incomingValue);
      continue;
    }
    merged[field] = mergeValue(merged[field], incomingValue, field, conflicts);
  }
  return merged;
}

function applyProtocolRefinements(message) {
  const normalized = structuredClone(message);
  const messageType = Number(normalized.protocol?.message_type);
  if (messageType === 1001 || messageType === 50001) normalized.type = "system";
  else if (messageType === 27 && normalized.images?.length) normalized.type = normalized.text ? "mixed" : "image";
  else if (messageType === 7 && normalized.text) normalized.type = normalized.images?.length ? "mixed" : "text";
  return normalized;
}

async function mergeMessages(projectRoot, profileId, group, messages, runId) {
  const { directory, metadata } = group.kind || group.conversation_type
    ? await ensureConversation(projectRoot, profileId, group)
    : await ensureGroup(projectRoot, profileId, group);
  const messagesPath = path.join(directory, "messages.jsonl");
  const conflictsPath = path.join(directory, "conflicts.jsonl");
  const existing = (await readJsonl(messagesPath, { optional: true, validate: validateRecord })).map(applyProtocolRefinements);
  const byId = new Map(existing.map((message) => [message.message_id, message]));
  let added = 0;
  let duplicate = 0;
  let conflictCount = 0;
  let skipped = 0;
  for (const message of messages) {
    if (!message.message_id) {
      skipped += 1;
      await appendJsonl(conflictsPath, {
        schema: "douyin-group-message.conflict.v1",
        conflict_id: randomId("conflict"),
        occurred_at: now(),
        run_id: runId,
        conversation_id: String(group.conversation_id),
        message_id: null,
        field: "message_id",
        existing_value: null,
        incoming_value: null,
        reason_code: "MESSAGE_ID_MISSING",
      });
      continue;
    }
    validateRecord(message);
    const current = byId.get(message.message_id);
    if (!current) {
      byId.set(message.message_id, message);
      added += 1;
      continue;
    }
    const conflicts = [];
    const merged = mergeMessage(current, message, conflicts);
    if (conflicts.length) {
      for (const conflict of conflicts) {
        await appendJsonl(conflictsPath, {
          schema: "douyin-group-message.conflict.v1",
          conflict_id: randomId("conflict"),
          occurred_at: now(),
          run_id: runId,
          conversation_id: String(group.conversation_id),
          message_id: message.message_id,
          field: conflict.field,
          existing_value: conflict.existing_value,
          incoming_value: conflict.incoming_value,
          reason_code: "NON_EMPTY_VALUE_MISMATCH",
        });
      }
      conflictCount += conflicts.length;
    } else {
      duplicate += 1;
    }
    byId.set(message.message_id, merged);
  }
  const output = [...byId.values()].sort((left, right) => {
    const leftTime = Date.parse(left.sent_at || left.collected_at);
    const rightTime = Date.parse(right.sent_at || right.collected_at);
    return leftTime - rightTime || left.message_id.localeCompare(right.message_id);
  });
  await atomicWriteJsonl(messagesPath, output);
  const timestamps = output.map((message) => message.sent_at).filter(Boolean).sort();
  const nextMetadata = {
    ...metadata,
    updated_at: now(),
    last_successful_run_id: runId,
    oldest_collected_at: timestamps[0] || metadata.oldest_collected_at,
    newest_collected_at: timestamps.at(-1) || metadata.newest_collected_at,
    message_count: output.length,
  };
  await atomicWriteJson(path.join(directory, metadata.schema === "douyin-message.conversation.v1" ? "conversation.json" : "group.json"), nextMetadata);
  return { directory, messagesPath, added, duplicate, conflicts: conflictCount, skipped, total: output.length };
}

async function createRun(projectRoot, profileId, groupIds, from, to) {
  const root = await ensureDataProfile(projectRoot, profileId);
  const runId = randomId("run");
  const timestamp = now();
  const run = {
    schema: "douyin-group-message.collection-run.v1",
    run_id: runId,
    profile_id: safeSegment(profileId),
    requested_at: timestamp,
    started_at: timestamp,
    completed_at: null,
    from,
    to,
    selected_groups: groupIds.map(String),
    status: "running",
    totals: { groups_selected: groupIds.length, groups_succeeded: 0, groups_failed: 0, messages_new: 0, messages_duplicate: 0, messages_conflict: 0, messages_skipped: 0 },
    groups: groupIds.map((conversationId) => ({ conversation_id: String(conversationId), status: "pending", messages_new: 0, messages_duplicate: 0, messages_conflict: 0, messages_skipped: 0, error_code: null, output_dir: null })),
  };
  validateRecord(run);
  const filename = path.join(root, "runs", `${runId}.json`);
  await atomicWriteJson(filename, run);
  return { run, filename };
}

async function updateRun(filename, run) {
  validateRecord(run, { path: filename });
  await atomicWriteJson(filename, run);
}

async function listInterruptedRuns(projectRoot, profileId) {
  const directory = path.join(dataProfileRoot(projectRoot, profileId), "runs");
  await ensureDir(directory);
  const entries = await fsp.readdir(directory, { withFileTypes: true });
  const runs = [];
  for (const entry of entries) {
    if (!entry.isFile() || !entry.name.endsWith(".json")) continue;
    const filename = path.join(directory, entry.name);
    const run = await readJson(filename);
    validateRecord(run, { path: filename });
    if (run.status === "running") runs.push({ run_id: run.run_id, started_at: run.started_at, selected_groups: run.selected_groups, completed_groups: run.groups.filter((group) => group.status === "completed").map((group) => group.conversation_id), run_file: filename });
  }
  return runs.sort((left, right) => left.started_at.localeCompare(right.started_at));
}

async function findMessage(projectRoot, profileId, conversationId, messageId) {
  let conversation;
  let directory;
  try {
    const index = await readConversationsIndex(projectRoot, profileId);
    conversation = index.conversations.find((item) => item.conversation_id === String(conversationId));
    if (conversation) ({ directory } = await ensureConversation(projectRoot, profileId, conversation));
  } catch (error) {
    if (error.code !== "CONVERSATION_INDEX_NOT_FOUND") throw error;
  }
  if (!conversation) {
    const legacy = await readGroupsIndex(projectRoot, profileId);
    conversation = legacy.groups.find((item) => item.conversation_id === String(conversationId));
    if (conversation) ({ directory } = await ensureGroup(projectRoot, profileId, conversation));
  }
  if (!conversation) throw new AppError("CONVERSATION_NOT_FOUND", `Conversation not found in the latest index: ${conversationId}.`);
  let messages = await readJsonl(path.join(directory, "messages.jsonl"), { optional: true, validate: validateRecord });
  let matches = messages.filter((message) => message.message_id === String(messageId));
  if (matches.length === 0 && conversation.kind) {
    const legacy = await readJson(path.join(dataProfileRoot(projectRoot, profileId), "groups.json"), { optional: true });
    const legacyGroup = legacy?.groups.find((item) => item.conversation_id === String(conversationId));
    if (legacyGroup) {
      const legacyLocation = await ensureGroup(projectRoot, profileId, legacyGroup);
      const legacyMessages = await readJsonl(path.join(legacyLocation.directory, "messages.jsonl"), { optional: true, validate: validateRecord });
      const legacyMatches = legacyMessages.filter((message) => message.message_id === String(messageId));
      if (legacyMatches.length) {
        directory = legacyLocation.directory;
        messages = legacyMessages;
        matches = legacyMatches;
      }
    }
  }
  if (matches.length !== 1) throw new AppError("MESSAGE_NOT_UNIQUE", `Expected one exported target message, found ${matches.length}.`, { conversation_id: conversationId, message_id: messageId });
  return { group: conversation, conversation, directory, message: matches[0] };
}

async function readConversationMessages(projectRoot, profileId, conversation) {
  const { directory } = await ensureConversation(projectRoot, profileId, conversation);
  const messages = await readJsonl(path.join(directory, "messages.jsonl"), { optional: true, validate: validateRecord });
  return { directory, messages };
}

async function appendReplyEvent(directory, event) {
  validateRecord(event);
  const filename = path.join(directory, "replies.jsonl");
  const existing = await readJsonl(filename, { optional: true, validate: validateRecord });
  if (existing.some((record) => record.event_id === event.event_id)) {
    throw new AppError("DUPLICATE_REPLY_EVENT", `Reply event already exists: ${event.event_id}.`);
  }
  await appendJsonl(filename, event);
  return filename;
}

function redactConflictValue(value) {
  if (value === null || value === undefined) return value;
  return { type: Array.isArray(value) ? "array" : typeof value, sha256: sha256(JSON.stringify(value)), length: JSON.stringify(value).length };
}

module.exports = {
  appendReplyEvent,
  conversationFolderName,
  createRun,
  dataProfileRoot,
  ensureDataProfile,
  ensureGroup,
  ensureConversation,
  findMessage,
  groupFolderName,
  listInterruptedRuns,
  mergeMessages,
  readConversationMessages,
  readGroupsIndex,
  readConversationsIndex,
  readLatestWarmup,
  redactConflictValue,
  updateRun,
  writeGroupsIndex,
  writeConversationsIndex,
  writeWarmupSnapshot,
};
