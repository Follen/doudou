"use strict";

const fs = require("node:fs");
const fsp = require("node:fs/promises");
const childProcess = require("node:child_process");
const path = require("node:path");
const {
  AppError,
  emit,
  now,
  parseTimestamp,
  randomId,
  readJson,
  requireString,
  sha256,
  sleep,
} = require("./core");
const { calibrateFingerprint, captureSessionFingerprint, collectConversation, detectLogin, discoverConversations, discoverGroups, launchSession, openChat, sendQuotedReply } = require("./adapter");
const { runDoctor, runSetup } = require("./doctor");
const { loadFingerprint } = require("./fingerprint");
const { createProfile, listProfiles, loadProfile, updateProfile, withProfileLock } = require("./profile");
const { runPageDoctor } = require("./page-doctor");
const { appendReplyEvent, createRun, findMessage, listInterruptedRuns, mergeMessages, readConversationMessages, readConversationsIndex, readGroupsIndex, updateRun, writeConversationsIndex, writeGroupsIndex } = require("./storage");
const { sendProtocolMessage } = require("./protocol");
const { defaultMessageWindow, resolveConversationTarget, resolveMessageTarget } = require("./resolver");
const { runWarmup } = require("./warmup");

function relative(projectRoot, filename) {
  return path.relative(projectRoot, filename).replaceAll("\\", "/");
}

function cliCommand(argumentsText) {
  return `node "${path.join(__dirname, "..", "bin", "douyin-message.js")}" ${argumentsText}`;
}

async function assertEnvironment(projectRoot) {
  const doctor = await runDoctor({ projectRoot, launchProbe: true });
  emit("doctor", { result: doctor });
  if (doctor.status !== "ok") {
    throw new AppError("DOCTOR_FAILED", "Environment doctor did not pass.", { status: doctor.status, issues: doctor.issues, repair_command: cliCommand("setup") });
  }
  return doctor;
}

async function prepareFingerprint(context, profileId, paths) {
  try {
    return await loadFingerprint(paths.fingerprint, { profileId });
  } catch (error) {
    if (error.code !== "FINGERPRINT_BASELINE_REQUIRED" || !context.args.auto_calibrate_fallback) throw error;
    const browser = context.args.browser || "chromium";
    emit("fingerprint_calibration_started", { profile_id: profileId, browser, headed: true, fallback: true });
    const record = await calibrateFingerprint({
      browserData: paths.browserData,
      fingerprintPath: paths.fingerprint,
      profileId,
      browser,
    });
    emit("fingerprint_calibration_completed", {
      profile_id: profileId,
      browser: record.browser,
      captured_at: record.captured_at,
      fingerprint_file: relative(context.projectRoot, paths.fingerprint),
      fallback: true,
    });
    return await loadFingerprint(paths.fingerprint, { profileId });
  }
}

async function launchHeadlessProfileSession(context, profileId, paths, options = {}) {
  const fingerprint = await prepareFingerprint(context, profileId, paths);
  return launchSession({
    browserData: paths.browserData,
    headless: true,
    browser: fingerprint.browser.browser_channel,
    fingerprint,
    fingerprintPath: paths.fingerprint,
    ...options,
  });
}

async function requireValidLogin(session, projectRoot, profileId) {
  const login = await detectLogin(session);
  if (!login.valid) {
    const errorCode = login.reason_code || (login.status === "verification_required" ? "VERIFICATION_REQUIRED" : "LOGIN_REQUIRED");
    await updateProfile(projectRoot, profileId, { login_status: login.status, last_error_code: errorCode });
    throw new AppError(errorCode, "The selected Profile requires headed login or verification.", { profile_id: profileId, repair_command: cliCommand(`profile login --profile ${profileId}`) });
  }
  await updateProfile(projectRoot, profileId, { login_status: "valid", last_login_verified_at: now(), last_error_code: null });
}

async function performWarmup(context, profileId, { allowStale }) {
  emit("warmup_started", { profile_id: profileId, allow_stale: allowStale, cache_max_age_ms: 7200000 });
  const result = await runWarmup({
    projectRoot: context.projectRoot,
    profileId,
    allowStale,
    terminateOwner: Boolean(context.args.terminate_owner),
  });
  emit("warmup_completed", {
    profile_id: profileId,
    warmup_source: result.warmup_source,
    completed_at: result.snapshot.completed_at,
    age_ms: result.age_ms,
    conversations: result.snapshot.conversations,
    files: result.files,
    live_error: result.live_error || null,
  });
  return result;
}

async function commandWarmup(context) {
  const profileId = requireString(context.args, "profile");
  const result = await performWarmup(context, profileId, { allowStale: Boolean(context.args.allow_stale) });
  emit("result", { command: "warmup", result: {
    schema: result.snapshot.schema,
    profile_id: profileId,
    status: "success",
    warmup_source: result.warmup_source,
    completed_at: result.snapshot.completed_at,
    age_ms: result.age_ms,
    conversations: result.snapshot.conversations,
    contract: result.snapshot.contract,
    files: result.files,
    headless: true,
  } });
}

async function commandDoctor(context) {
  const result = await runDoctor({ projectRoot: context.projectRoot, launchProbe: !context.args.skip_launch });
  emit("result", { command: "doctor", result });
  if (result.status === "failed") process.exitCode = 2;
}

async function commandSetup(context) {
  await runSetup({ skillRoot: context.skillRoot, onProgress: (progress) => emit("setup_progress", progress) });
  const result = await runDoctor({ projectRoot: context.projectRoot, launchProbe: true });
  emit("result", { command: "setup", result });
  if (result.status !== "ok") throw new AppError("SETUP_VERIFICATION_FAILED", "Setup completed but doctor still reports issues.", { issues: result.issues });
}

async function commandProfileList(context) {
  const profiles = await listProfiles(context.projectRoot);
  emit("result", { command: "profile list", result: { count: profiles.length, profiles } });
}

async function commandProfileCreate(context) {
  const profileId = requireString(context.args, "profile");
  const result = await createProfile(context.projectRoot, profileId, context.args.display_name || profileId);
  emit("result", { command: "profile create", result: { profile: result.metadata, directory: relative(context.projectRoot, result.paths.directory) } });
}

async function commandProfileLogin(context) {
  await assertEnvironment(context.projectRoot);
  const profileId = requireString(context.args, "profile");
  const browser = context.args.browser || "chromium";
  if (!new Set(["chromium", "chrome"]).has(browser)) throw new AppError("INVALID_BROWSER", "--browser must be chromium or chrome.");
  const timeout = Number(context.args.timeout || 600000);
  await withProfileLock(context.projectRoot, profileId, { terminateOwner: Boolean(context.args.terminate_owner) }, async () => {
    const { paths } = await loadProfile(context.projectRoot, profileId);
    const session = await launchSession({ browserData: paths.browserData, headless: false, browser });
    try {
      await openChat(session);
      emit("login_waiting", { profile_id: profileId, headed: true, timeout_ms: timeout });
      const minimumOpenUntil = Date.now() + 15000;
      const deadline = Date.now() + timeout;
      while (Date.now() < deadline) {
        if (session.page.isClosed()) {
          await updateProfile(context.projectRoot, profileId, { login_status: "login_required", last_error_code: "LOGIN_CANCELLED" });
          throw new AppError("LOGIN_CANCELLED", "The headed login window was closed before verification completed.", { profile_id: profileId });
        }
        const login = await detectLogin(session);
        if (login.valid) {
          if (Date.now() < minimumOpenUntil) {
            await sleep(1000);
            continue;
          }
          await session.page.waitForTimeout(3000);
          const stableLogin = await detectLogin(session);
          if (!stableLogin.valid) {
            await sleep(1000);
            continue;
          }
          const fingerprint = await captureSessionFingerprint({ session, profileId, fingerprintPath: paths.fingerprint, browser });
          const profile = await updateProfile(context.projectRoot, profileId, {
            login_status: "valid",
            last_login_verified_at: now(),
            fingerprint_status: "valid",
            fingerprint_captured_at: fingerprint.captured_at,
            last_error_code: null,
          });
          emit("result", { command: "profile login", result: {
            profile,
            verified: true,
            fingerprint: {
              schema: fingerprint.schema,
              browser: fingerprint.browser,
              captured_at: fingerprint.captured_at,
              file: relative(context.projectRoot, paths.fingerprint),
            },
          } });
          return;
        }
        await sleep(1500);
      }
      await updateProfile(context.projectRoot, profileId, { login_status: "login_required", last_error_code: "LOGIN_TIMEOUT" });
      throw new AppError("LOGIN_TIMEOUT", "Headed login was not verified before the timeout.", { profile_id: profileId });
    } finally {
      await session.context.close().catch(() => {});
    }
  });
}

async function commandProfileCalibrate(context) {
  await assertEnvironment(context.projectRoot);
  const profileId = requireString(context.args, "profile");
  const browser = context.args.browser || "chromium";
  if (!new Set(["chromium", "chrome"]).has(browser)) throw new AppError("INVALID_BROWSER", "--browser must be chromium or chrome.");
  await withProfileLock(context.projectRoot, profileId, { terminateOwner: Boolean(context.args.terminate_owner) }, async () => {
    const { paths } = await loadProfile(context.projectRoot, profileId);
    emit("fingerprint_calibration_started", { profile_id: profileId, browser, headed: true, fallback: false });
    const fingerprint = await calibrateFingerprint({
      browserData: paths.browserData,
      fingerprintPath: paths.fingerprint,
      profileId,
      browser,
    });
    const profile = await updateProfile(context.projectRoot, profileId, {
      fingerprint_status: "valid",
      fingerprint_captured_at: fingerprint.captured_at,
      last_error_code: null,
    });
    emit("result", { command: "profile calibrate", result: {
      profile_id: profile.profile_id,
      status: "valid",
      headed: true,
      browser: fingerprint.browser,
      captured_at: fingerprint.captured_at,
      fingerprint_file: relative(context.projectRoot, paths.fingerprint),
    } });
  });
}

async function commandProfileStatus(context) {
  await assertEnvironment(context.projectRoot);
  const profileId = requireString(context.args, "profile");
  await withProfileLock(context.projectRoot, profileId, { terminateOwner: Boolean(context.args.terminate_owner) }, async () => {
    const { paths } = await loadProfile(context.projectRoot, profileId);
    const session = await launchHeadlessProfileSession(context, profileId, paths);
    try {
      await openChat(session);
      const login = await detectLogin(session);
      const profile = await updateProfile(context.projectRoot, profileId, {
        login_status: login.status,
        last_login_verified_at: login.valid ? now() : null,
        last_error_code: login.valid ? null : login.reason_code || "LOGIN_REQUIRED",
      });
      emit("result", { command: "profile status", result: { profile, login, headless: true } });
    } finally {
      await session.context.close();
    }
  });
}

async function commandGroupsRefresh(context) {
  await assertEnvironment(context.projectRoot);
  const profileId = requireString(context.args, "profile");
  await withProfileLock(context.projectRoot, profileId, { terminateOwner: Boolean(context.args.terminate_owner) }, async () => {
    const { paths } = await loadProfile(context.projectRoot, profileId);
    const session = await launchHeadlessProfileSession(context, profileId, paths);
    try {
      await openChat(session);
      await requireValidLogin(session, context.projectRoot, profileId);
      const groups = await discoverGroups(session);
      const index = await writeGroupsIndex(context.projectRoot, profileId, groups);
      emit("result", { command: "groups refresh", result: { ...index, headless: true } });
    } finally {
      await session.context.close();
    }
  });
}

async function commandConversationsRefresh(context) {
  const profileId = requireString(context.args, "profile");
  const kind = String(context.args.kind || "all").toLowerCase();
  if (!new Set(["all", "group", "direct"]).has(kind)) throw new AppError("INVALID_CONVERSATION_KIND", "--kind must be all, group or direct.");
  const warmup = await performWarmup(context, profileId, { allowStale: false });
  const conversations = kind === "all" ? warmup.index.conversations : warmup.index.conversations.filter((conversation) => conversation.kind === kind);
  emit("result", { command: "conversations refresh", result: { ...warmup.index, count: conversations.length, conversations, kind_filter: kind, warmup_source: warmup.warmup_source, headless: true } });
}

async function commandConversationsResolve(context) {
  const profileId = requireString(context.args, "profile");
  const target = context.args.target || context.args.recipient || context.args.conversation;
  if (typeof target !== "string" || !target.trim()) throw new AppError("MISSING_ARGUMENT", "Missing required argument --target.", { argument: "target" });
  const warmup = await performWarmup(context, profileId, { allowStale: true });
  const resolved = resolveConversationTarget(warmup.index, target);
  emit("result", { command: "conversations resolve", result: {
    profile_id: profileId,
    target,
    warmup_source: warmup.warmup_source,
    matched_by: resolved.matched_by,
    conversation: resolved.conversation,
  } });
}

function parseGroupSelection(value, count) {
  const text = String(value || "").trim();
  if (!text) throw new AppError("EMPTY_GROUP_SELECTION", "A group selection is required.");
  if (/^(?:全部|all)$/i.test(text)) return Array.from({ length: count }, (_, index) => index + 1);
  const selected = [];
  const seen = new Set();
  for (const token of text.split(/[,，]/).map((item) => item.trim()).filter(Boolean)) {
    const match = token.match(/^(\d+)(?:\s*-\s*(\d+))?$/);
    if (!match) throw new AppError("INVALID_GROUP_SELECTION", `Invalid group selection token: ${token}.`, { token });
    const start = Number(match[1]);
    const end = Number(match[2] || match[1]);
    if (start > end) throw new AppError("INVALID_GROUP_RANGE", `Group range must be ascending: ${token}.`, { token });
    for (let index = start; index <= end; index += 1) {
      if (index < 1 || index > count) throw new AppError("GROUP_NUMBER_OUT_OF_RANGE", `Group number ${index} is outside 1-${count}.`, { number: index, count });
      if (!seen.has(index)) {
        seen.add(index);
        selected.push(index);
      }
    }
  }
  if (!selected.length) throw new AppError("EMPTY_GROUP_SELECTION", "A group selection is required.");
  return selected;
}

async function commandGroupsSelect(context) {
  const profileId = requireString(context.args, "profile");
  const selection = requireString(context.args, "selection");
  const refreshedAt = requireString(context.args, "refreshed_at");
  const index = await readGroupsIndex(context.projectRoot, profileId);
  if (index.refreshed_at !== refreshedAt) throw new AppError("GROUP_SELECTION_STALE", "The group index was refreshed after these numbers were shown.", { expected: index.refreshed_at, received: refreshedAt });
  if (index.count === 0) throw new AppError("NO_GROUPS_AVAILABLE", "No group conversations are available for selection.");
  const numbers = parseGroupSelection(selection, index.count);
  const groups = numbers.map((number) => ({ number, ...index.groups[number - 1] }));
  emit("result", { command: "groups select", result: { profile_id: index.profile_id, refreshed_at: index.refreshed_at, count: groups.length, groups } });
}

async function commandConversationsSelect(context) {
  const profileId = requireString(context.args, "profile");
  const selection = requireString(context.args, "selection");
  const refreshedAt = requireString(context.args, "refreshed_at");
  const index = await readConversationsIndex(context.projectRoot, profileId);
  if (index.refreshed_at !== refreshedAt) throw new AppError("CONVERSATION_SELECTION_STALE", "The conversation index was refreshed after these numbers were shown.", { expected: index.refreshed_at, received: refreshedAt });
  if (index.count === 0) throw new AppError("NO_CONVERSATIONS_AVAILABLE", "No conversations are available for selection.");
  const numbers = parseGroupSelection(selection, index.count);
  const conversations = numbers.map((number) => ({ number, ...index.conversations[number - 1] }));
  emit("result", { command: "conversations select", result: { profile_id: index.profile_id, refreshed_at: index.refreshed_at, count: conversations.length, conversations } });
}

function applyGroupResult(run, conversationId, status, stats = {}, errorCode = null, outputDir = null) {
  const item = run.groups.find((group) => group.conversation_id === String(conversationId));
  item.status = status;
  item.messages_new = stats.added || 0;
  item.messages_duplicate = stats.duplicate || 0;
  item.messages_conflict = stats.conflicts || 0;
  item.messages_skipped = stats.skipped || 0;
  item.error_code = errorCode;
  item.output_dir = outputDir;
  if (status === "completed") run.totals.groups_succeeded += 1;
  if (status === "failed") run.totals.groups_failed += 1;
  run.totals.messages_new += item.messages_new;
  run.totals.messages_duplicate += item.messages_duplicate;
  run.totals.messages_conflict += item.messages_conflict;
  run.totals.messages_skipped += item.messages_skipped;
}

function resolveMessageWindow(args) {
  if (!args.from && !args.to) return defaultMessageWindow();
  if (!args.from || !args.to) throw new AppError("MISSING_ARGUMENT", "Use --from and --to together.", { arguments: ["from", "to"] });
  const from = parseTimestamp(String(args.from), "from");
  const to = parseTimestamp(String(args.to), "to");
  if (from > to) throw new AppError("INVALID_TIME_RANGE", "--from must not be later than --to.");
  return { from, to, defaulted: false };
}

async function collectIndexedConversations(context, profileId, groups, window, { emitResult = true } = {}) {
  const groupIds = groups.map((group) => group.conversation_id);
  return withProfileLock(context.projectRoot, profileId, { terminateOwner: Boolean(context.args.terminate_owner) }, async () => {
    const interrupted = await listInterruptedRuns(context.projectRoot, profileId);
    if (interrupted.length) emit("interrupted_runs", { count: interrupted.length, runs: interrupted.map((run) => ({ ...run, run_file: relative(context.projectRoot, run.run_file) })) });
    const { paths } = await loadProfile(context.projectRoot, profileId);
    const session = await launchHeadlessProfileSession(context, profileId, paths);
    const { run, filename } = await createRun(context.projectRoot, profileId, groupIds, window.from, window.to);
    try {
      await openChat(session);
      await requireValidLogin(session, context.projectRoot, profileId);
      for (let position = 0; position < groups.length; position += 1) {
        const group = groups[position];
        run.groups[position].status = "running";
        await updateRun(filename, run);
        emit("conversation_progress", { current: position + 1, total: groups.length, conversation_id: group.conversation_id, kind: group.kind || "group", display_name: group.display_name, status: "running" });
        try {
          const collected = await collectConversation(session, { profileId, group, from: window.from, to: window.to, runId: run.run_id, maxScrolls: Number(context.args.max_scrolls || 80) });
          const stats = await mergeMessages(context.projectRoot, profileId, group, collected.messages, run.run_id);
          stats.skipped += collected.unresolved.length;
          applyGroupResult(run, group.conversation_id, "completed", stats, null, relative(context.projectRoot, stats.directory));
          emit("conversation_progress", { current: position + 1, total: groups.length, conversation_id: group.conversation_id, kind: group.kind || "group", display_name: group.display_name, status: "completed", stats: run.groups[position] });
        } catch (error) {
          applyGroupResult(run, group.conversation_id, "failed", {}, error.code || "COLLECTION_FAILED");
          emit("conversation_progress", { current: position + 1, total: groups.length, conversation_id: group.conversation_id, kind: group.kind || "group", display_name: group.display_name, status: "failed", error_code: error.code || "COLLECTION_FAILED", message: error.message });
          await openChat(session).catch(() => {});
        }
        await updateRun(filename, run);
      }
      run.completed_at = now();
      run.status = run.totals.groups_failed === 0 ? "completed" : run.totals.groups_succeeded > 0 ? "partial" : "failed";
      await updateRun(filename, run);
      const result = { run, run_file: relative(context.projectRoot, filename), time_range_defaulted: window.defaulted, headless: true };
      if (emitResult) emit("result", { command: "messages collect", result });
      return result;
    } catch (error) {
      run.completed_at = now();
      run.status = run.totals.groups_succeeded > 0 ? "partial" : "failed";
      await updateRun(filename, run);
      throw error;
    } finally {
      await session.context.close();
    }
  });
}

async function commandCollect(context) {
  const profileId = requireString(context.args, "profile");
  const warmup = await performWarmup(context, profileId, { allowStale: true });
  const window = resolveMessageWindow(context.args);
  const target = context.args.conversation || context.args.recipient;
  let groups;
  if (typeof target === "string" && target.trim()) {
    groups = [resolveConversationTarget(warmup.index, target).conversation];
  } else {
    const conversationArgument = context.args.conversations || context.args.groups;
    if (typeof conversationArgument !== "string" || !conversationArgument.trim()) throw new AppError("MISSING_ARGUMENT", "Use --conversation NAME or --conversations ID,ID.", { arguments: ["conversation", "conversations"] });
    const ids = conversationArgument.split(",").map((value) => value.trim()).filter(Boolean);
    groups = ids.map((id) => {
      const conversation = warmup.index.conversations.find((candidate) => candidate.conversation_id === id);
      if (!conversation) throw new AppError("CONVERSATION_SELECTION_STALE", `Conversation ${id} is not in the warmup index.`, { refreshed_at: warmup.index.refreshed_at });
      return conversation;
    });
  }
  return collectIndexedConversations(context, profileId, groups, window);
}

async function commandMessagesCollect(context) {
  return commandCollect(context);
}

function summarizeText(text, maxLength = 120) {
  const compact = String(text || "").replace(/\s+/g, " ").trim();
  return compact.length > maxLength ? `${compact.slice(0, maxLength - 3)}...` : compact;
}

function replyPreview(target, replyText) {
  return {
    conversation_id: target.group.conversation_id,
    conversation_name: target.group.display_name,
    target_message_id: target.message.message_id,
    sender: target.message.sender,
    sent_at: target.message.sent_at,
    message_summary: summarizeText(target.message.text || `[${target.message.type}]`),
    reply_text: replyText,
  };
}

function replyEvent({ planId, taskId, profileId, conversationId, messageId, eventType, replyText, status, reasonCode = null, message = null, sentMessageId = null }) {
  return {
    schema: "douyin-group-message.reply-event.v1",
    event_id: randomId("reply-event"),
    plan_id: planId,
    task_id: taskId,
    occurred_at: now(),
    profile_id: profileId,
    conversation_id: conversationId,
    target_message_id: messageId,
    event_type: eventType,
    reply_text: replyText,
    attempt: 1,
    result: { status, sent_message_id: sentMessageId, reason_code: reasonCode, message },
  };
}

async function executeReplyTask(context, session, profileId, task, planId, taskId) {
  const target = await findMessage(context.projectRoot, profileId, task.conversation_id, task.message_id);
  const preview = replyPreview(target, task.text);
  await appendReplyEvent(target.directory, replyEvent({ planId, taskId, profileId, conversationId: task.conversation_id, messageId: task.message_id, eventType: "send_started", replyText: task.text, status: "pending" }));
  try {
    const sent = await sendQuotedReply(session, {
      group: target.group,
      messageId: task.message_id,
      targetText: target.message.text,
      replyText: task.text,
    });
    await appendReplyEvent(target.directory, replyEvent({ planId, taskId, profileId, conversationId: task.conversation_id, messageId: task.message_id, eventType: "sent", replyText: task.text, status: "sent", sentMessageId: sent.sent_message_id }));
    return { preview, status: "sent", sent };
  } catch (error) {
    await appendReplyEvent(target.directory, replyEvent({ planId, taskId, profileId, conversationId: task.conversation_id, messageId: task.message_id, eventType: "failed", replyText: task.text, status: "failed", reasonCode: error.code || "SEND_FAILED", message: error.message }));
    throw error;
  }
}

async function commandReply(context) {
  const profileId = requireString(context.args, "profile");
  await performWarmup(context, profileId, { allowStale: !context.args.confirm });
  const task = { conversation_id: requireString(context.args, "conversation_id"), message_id: requireString(context.args, "message_id"), text: requireString(context.args, "text") };
  const target = await findMessage(context.projectRoot, profileId, task.conversation_id, task.message_id);
  const preview = replyPreview(target, task.text);
  const planId = context.args.plan_id || `reply-plan-${sha256(JSON.stringify({ profileId, ...task })).slice(0, 16)}`;
  const taskId = `${planId}-1`;
  if (!context.args.confirm) {
    await appendReplyEvent(target.directory, replyEvent({ planId, taskId, profileId, conversationId: task.conversation_id, messageId: task.message_id, eventType: "planned", replyText: task.text, status: "pending" }));
    emit("result", { command: "reply", result: { status: "confirmation_required", plan_id: planId, preview, retry_arguments: ["--confirm", "--plan-id", planId] } });
    return;
  }
  await assertEnvironment(context.projectRoot);
  await withProfileLock(context.projectRoot, profileId, { terminateOwner: Boolean(context.args.terminate_owner) }, async () => {
    await appendReplyEvent(target.directory, replyEvent({ planId, taskId, profileId, conversationId: task.conversation_id, messageId: task.message_id, eventType: "confirmed", replyText: task.text, status: "pending" }));
    const { paths } = await loadProfile(context.projectRoot, profileId);
    const session = await launchHeadlessProfileSession(context, profileId, paths);
    try {
      await openChat(session);
      await requireValidLogin(session, context.projectRoot, profileId);
      const result = await executeReplyTask(context, session, profileId, task, planId, taskId);
      emit("result", { command: "reply", result: { plan_id: planId, ...result, headless: true } });
    } finally {
      await session.context.close();
    }
  });
}

async function commandReplyBatch(context) {
  const profileId = requireString(context.args, "profile");
  await performWarmup(context, profileId, { allowStale: !context.args.confirm_batch });
  const planPath = path.resolve(requireString(context.args, "plan"));
  const plan = await readJson(planPath);
  if (!Array.isArray(plan.tasks) || plan.tasks.length === 0) throw new AppError("INVALID_BATCH_PLAN", "Batch plan must contain a non-empty tasks array.");
  const tasks = [];
  for (const item of plan.tasks) {
    if (!item.conversation_id || !item.message_id || !item.text) throw new AppError("INVALID_BATCH_PLAN", "Each batch task requires conversation_id, message_id and text.");
    const target = await findMessage(context.projectRoot, profileId, item.conversation_id, item.message_id);
    tasks.push({ task: item, target, preview: replyPreview(target, item.text) });
  }
  const planId = plan.plan_id || `reply-plan-${sha256(JSON.stringify({ profileId, tasks: plan.tasks })).slice(0, 16)}`;
  if (!context.args.confirm_batch) {
    for (let index = 0; index < tasks.length; index += 1) {
      const item = tasks[index];
      await appendReplyEvent(item.target.directory, replyEvent({ planId, taskId: `${planId}-${index + 1}`, profileId, conversationId: item.task.conversation_id, messageId: item.task.message_id, eventType: "planned", replyText: item.task.text, status: "pending" }));
    }
    emit("result", { command: "reply-batch", result: { status: "confirmation_required", plan_id: planId, tasks: tasks.map((item) => item.preview), retry_argument: "--confirm-batch" } });
    return;
  }
  await assertEnvironment(context.projectRoot);
  await withProfileLock(context.projectRoot, profileId, { terminateOwner: Boolean(context.args.terminate_owner) }, async () => {
    const { paths } = await loadProfile(context.projectRoot, profileId);
    const session = await launchHeadlessProfileSession(context, profileId, paths);
    const results = [];
    try {
      await openChat(session);
      await requireValidLogin(session, context.projectRoot, profileId);
      for (let index = 0; index < tasks.length; index += 1) {
        const item = tasks[index];
        const taskId = `${planId}-${index + 1}`;
        await appendReplyEvent(item.target.directory, replyEvent({ planId, taskId, profileId, conversationId: item.task.conversation_id, messageId: item.task.message_id, eventType: "confirmed", replyText: item.task.text, status: "pending" }));
        emit("reply_progress", { current: index + 1, total: tasks.length, task_id: taskId, status: "running" });
        try {
          const result = await executeReplyTask(context, session, profileId, item.task, planId, taskId);
          results.push(result);
          emit("reply_progress", { current: index + 1, total: tasks.length, task_id: taskId, status: "sent" });
        } catch (error) {
          results.push({ preview: item.preview, status: "failed", error_code: error.code || "SEND_FAILED", message: error.message });
          for (let later = index + 1; later < tasks.length; later += 1) {
            const skipped = tasks[later];
            await appendReplyEvent(skipped.target.directory, replyEvent({ planId, taskId: `${planId}-${later + 1}`, profileId, conversationId: skipped.task.conversation_id, messageId: skipped.task.message_id, eventType: "skipped", replyText: skipped.task.text, status: "skipped", reasonCode: "BATCH_STOPPED" }));
            results.push({ preview: skipped.preview, status: "skipped", error_code: "BATCH_STOPPED" });
          }
          break;
        }
      }
      emit("result", { command: "reply-batch", result: { plan_id: planId, results, headless: true } });
    } finally {
      await session.context.close();
    }
  });
}

async function protocolPreview(context, mode) {
  const profileId = requireString(context.args, "profile");
  const text = requireString(context.args, "text");
  const warmup = await performWarmup(context, profileId, { allowStale: !context.args.confirm });
  const requestedConversation = context.args.conversation_id || context.args.recipient || context.args.conversation;
  const resolution = resolveConversationTarget(warmup.index, requestedConversation);
  const conversation = resolution.conversation;
  const conversationId = conversation.conversation_id;
  let target = null;
  let messageMatchedBy = null;
  if (mode === "reference_reply") {
    if (context.args.message_id) {
      target = (await findMessage(context.projectRoot, profileId, conversationId, context.args.message_id)).message;
      messageMatchedBy = "id";
    } else {
      const messageText = requireString(context.args, "message_text");
      const window = resolveMessageWindow(context.args);
      await collectIndexedConversations(context, profileId, [conversation], window, { emitResult: false });
      const stored = await readConversationMessages(context.projectRoot, profileId, conversation);
      const resolvedMessage = resolveMessageTarget(stored.messages, messageText);
      target = resolvedMessage.message;
      messageMatchedBy = resolvedMessage.matched_by;
    }
  }
  const planId = `protocol-plan-${sha256(JSON.stringify({ profileId, conversationId, messageId: target?.message_id || null, text, mode })).slice(0, 16)}`;
  const preview = {
    mode,
    profile_id: profileId,
    conversation_id: conversationId,
    conversation_type: conversation.conversation_type,
    kind: conversation.kind,
    conversation_name: conversation.display_name,
    target_message_id: target?.message_id || null,
    target_sender: target?.sender || null,
    target_text: target ? summarizeText(target.text || `[${target.type}]`) : null,
    text,
    headless: true,
    warmup_source: warmup.warmup_source,
    conversation_matched_by: resolution.matched_by,
    message_matched_by: messageMatchedBy,
  };
  return { profileId, conversationId, text, target, planId, preview, warmup };
}

async function executeProtocolCommand(context, mode) {
  if (context.args.confirm && !/^protocol-plan-[0-9a-f]{16}$/.test(String(context.args.plan_id || ""))) {
    throw new AppError("PROTOCOL_PLAN_MISMATCH", "--plan-id must be the protocol plan returned by the current preview.", { received: context.args.plan_id || null });
  }
  const plan = await protocolPreview(context, mode);
  if (!context.args.confirm) {
    emit("result", { command: mode === "reference_reply" ? "protocol reply" : "protocol send", result: { status: "confirmation_required", plan_id: plan.planId, preview: plan.preview, retry_arguments: ["--confirm", "--plan-id", plan.planId] } });
    return;
  }
  if (context.args.plan_id !== plan.planId) throw new AppError("PROTOCOL_PLAN_MISMATCH", "--plan-id must match the current protocol-send preview.", { expected: plan.planId, received: context.args.plan_id || null });
  await assertEnvironment(context.projectRoot);
  const result = await sendProtocolMessage({
    dataRoot: context.projectRoot,
    profileId: plan.profileId,
    conversationId: plan.conversationId,
    text: plan.text,
    target: plan.target,
    terminateOwner: Boolean(context.args.terminate_owner),
  });
  emit("result", {
    command: mode === "reference_reply" ? "protocol reply" : "protocol send",
    result: {
      status: "sent",
      plan_id: plan.planId,
      directory: relative(context.projectRoot, result.directory),
      receipt: result.receipt,
      verification: result.verification,
    },
  });
}

async function commandProtocolSend(context) {
  return executeProtocolCommand(context, "plain");
}

async function commandProtocolReply(context) {
  return executeProtocolCommand(context, "reference_reply");
}

async function commandProtocolCapture(context) {
  const profileId = requireString(context.args, "profile");
  await assertEnvironment(context.projectRoot);
  const script = path.join(context.skillRoot, "scripts", "capture-headed-send.js");
  const args = [script, "--data-root", context.projectRoot, "--profile", profileId];
  if (context.args.terminate_owner) args.push("--terminate-owner");
  const status = await new Promise((resolve, reject) => {
    const child = childProcess.spawn(process.execPath, args, { cwd: context.skillRoot, stdio: "inherit", windowsHide: true });
    child.once("error", reject);
    child.once("exit", (code) => resolve(code));
  });
  if (status !== 0) throw new AppError("PROTOCOL_CAPTURE_FAILED", "The headed protocol capture process failed.", { exit_code: status });
}

async function commandPageDoctor(context) {
  await assertEnvironment(context.projectRoot);
  const profileId = requireString(context.args, "profile");
  await withProfileLock(context.projectRoot, profileId, { terminateOwner: Boolean(context.args.terminate_owner) }, async () => {
    const { paths } = await loadProfile(context.projectRoot, profileId);
    const fingerprint = await prepareFingerprint(context, profileId, paths);
    const result = await runPageDoctor({
      projectRoot: context.projectRoot,
      profileId,
      browserData: paths.browserData,
      fingerprint,
      fingerprintPath: paths.fingerprint,
      baselinePath: context.args.baseline || null,
    });
    const codeCounts = {};
    const severityCounts = { error: 0, warning: 0, info: 0 };
    for (const finding of result.doctor.findings) {
      codeCounts[finding.code] = (codeCounts[finding.code] || 0) + 1;
      severityCounts[finding.severity] = (severityCounts[finding.severity] || 0) + 1;
    }
    emit("result", { command: "page-doctor", result: {
      schema: result.doctor.schema,
      doctor_run_id: result.doctor.doctor_run_id,
      profile_id: result.doctor.profile_id,
      status: result.doctor.status,
      bundle_fingerprint: result.doctor.bundle_fingerprint,
      groups_observed: result.doctor.groups_observed,
      diagnostic_directory: relative(context.projectRoot, result.directory),
      artifacts: result.doctor.artifacts,
      findings_count: result.doctor.findings.length,
      finding_codes: codeCounts,
      finding_severity: severityCounts,
      findings_preview: result.doctor.findings.slice(0, 20),
      headless: true,
    } });
  });
}

module.exports = {
  commandCollect,
  commandConversationsRefresh,
  commandConversationsResolve,
  commandConversationsSelect,
  commandDoctor,
  commandGroupsRefresh,
  commandGroupsSelect,
  commandMessagesCollect,
  commandPageDoctor,
  commandProfileCreate,
  commandProfileCalibrate,
  commandProfileList,
  commandProfileLogin,
  commandProfileStatus,
  commandProtocolCapture,
  commandProtocolReply,
  commandProtocolSend,
  commandReply,
  commandReplyBatch,
  commandSetup,
  commandWarmup,
  collectIndexedConversations,
  performWarmup,
  replyPreview,
  parseGroupSelection,
};
