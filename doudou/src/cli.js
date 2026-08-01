"use strict";

const path = require("node:path");
const { AppError, emit, parseArgs, resolveDataRoot } = require("./core");
const commands = require("./commands");

const HELP = `doudou

Usage:
  doudou doctor [--data-root PATH] [--skip-launch]
  doudou setup [--data-root PATH]
  doudou profile list
  doudou profile create --profile NAME [--display-name NAME]
  doudou profile login --profile NAME [--browser chromium|chrome] [--timeout MS]
  doudou profile calibrate --profile NAME [--browser chromium|chrome]
  doudou profile status --profile NAME
  doudou warmup --profile NAME [--allow-stale]
  doudou conversations refresh --profile NAME [--kind all|group|direct]
  doudou conversations resolve --profile NAME --target ID|NAME
  doudou conversations select --profile NAME --refreshed-at RFC3339 --selection 1,3-5|全部
  doudou messages collect --profile NAME (--conversation NAME|--conversations ID,ID) [--from RFC3339 --to RFC3339] [--max-pages N]
  doudou protocol capture --profile NAME
  doudou protocol send --profile NAME (--recipient NAME|--conversation-id ID) (--text TEXT [--at MEMBER...]|--image FILE) [--confirm --plan-id ID]
  doudou protocol send-batch --profile NAME (--recipients NAME,NAME|--conversation-ids ID,ID) --text TEXT [--confirm --plan-id ID]
  doudou protocol reply --profile NAME (--recipient NAME|--conversation-id ID) (--message-text TEXT|--message-id ID) --text TEXT [--from RFC3339 --to RFC3339] [--confirm]
  doudou reply-batch --profile NAME --plan FILE [--confirm-batch --plan-id ID]
  doudou page-doctor --profile NAME [--baseline FILE]

Data layout:
  profile/<name>/browser-data/                                  persistent login state
  profile/<name>/fingerprint.json                              headed fingerprint baseline
  message/profiles/<name>/conversations/                       collected production messages
  message/profiles/<name>/warmup/                             current and dated warmup snapshots
  message/profiles/<name>/runs/                                collection records
  message/profiles/<name>/sends/YYYY-MM-DD/<time>__<client-id>/ send receipts
  message/profiles/<name>/diagnostics/                         optional capture/doctor records

Browser mode:
  Login, fingerprint calibration and manual verification are headed. Doctor,
  warmup, status, conversations, collect, reply and page-doctor use headless
  Chromium with the persisted Profile fingerprint. Collection, send and reply
  automatically run warmup first. Add --auto-calibrate-fallback to preauthorize
  one headed calibration for the current task when its baseline is unavailable.
`;

async function runCli(argv) {
  const args = parseArgs(argv);
  const [command, subcommand] = args._;
  if (!command || command === "help" || args.help) {
    process.stdout.write(HELP);
    return;
  }
  const context = {
    args,
    projectRoot: resolveDataRoot(args.data_root || args.project_root),
    skillRoot: path.resolve(__dirname, ".."),
  };
  const routes = new Map([
    ["doctor", commands.commandDoctor],
    ["setup", commands.commandSetup],
    ["warmup", commands.commandWarmup],
    ["profile:list", commands.commandProfileList],
    ["profile:create", commands.commandProfileCreate],
    ["profile:login", commands.commandProfileLogin],
    ["profile:calibrate", commands.commandProfileCalibrate],
    ["profile:status", commands.commandProfileStatus],
    ["groups:refresh", commands.commandGroupsRefresh],
    ["groups:select", commands.commandGroupsSelect],
    ["conversations:refresh", commands.commandConversationsRefresh],
    ["conversations:resolve", commands.commandConversationsResolve],
    ["conversations:select", commands.commandConversationsSelect],
    ["collect", commands.commandCollect],
    ["messages:collect", commands.commandMessagesCollect],
    ["reply", commands.commandReply],
    ["reply-batch", commands.commandReplyBatch],
    ["page-doctor", commands.commandPageDoctor],
    ["protocol:capture", commands.commandProtocolCapture],
    ["protocol:send", commands.commandProtocolSend],
    ["protocol:send-batch", commands.commandProtocolSendBatch],
    ["protocol:reply", commands.commandProtocolReply],
  ]);
  const key = new Set(["profile", "groups", "conversations", "messages", "protocol"]).has(command) ? `${command}:${subcommand || ""}` : command;
  const handler = routes.get(key);
  if (!handler) throw new AppError("UNKNOWN_COMMAND", `Unknown command: ${args._.join(" ")}.`, { help: HELP }, 2);
  emit("command_started", { command: key, data_root: context.projectRoot });
  await handler(context);
}

module.exports = { HELP, runCli };
