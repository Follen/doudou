//! `cf`-style self-describing CLI introspection: `cli search` and `cli schema`.
//!
//! All command paths, aliases, flags, defaults, possible values, and help text are extracted
//! directly from `Cli::command()` so discovery output can never drift from actual CLI parsing.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use clap::{Arg, ArgAction, Command, CommandFactory as _};
use doudou_core::{DoudouError, Result};
use serde::Serialize;
use serde_json::{Value, json};

use crate::args::Cli;

/// Schema version identifier emitted by `doudou cli schema`.
pub const CLI_SCHEMA_VERSION: &str = "doudou.cli-schema.v1";

/// Machine-readable description of a single CLI argument or flag.
#[derive(Debug, Clone, Serialize)]
pub struct ArgSchema {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub long: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub short: Option<char>,
    pub positional: bool,
    pub required: bool,
    pub takes_value: bool,
    pub multiple: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value_name: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub default_values: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub possible_values: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub help: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub long_help: Option<String>,
}

/// Machine-readable description of a CLI command or subcommand.
#[derive(Debug, Clone, Serialize)]
pub struct CommandSchema {
    pub path: String,
    pub name: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub aliases: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub about: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub long_about: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub examples: Option<String>,
    pub semantics: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub broker_command: Option<String>,
    pub two_phase_write: bool,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub related_error_codes: Vec<String>,
    pub args: Vec<ArgSchema>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub subcommands: Vec<CommandSchema>,
}

/// Stable error code catalog entry included in `cli schema` and searchable via `cli search`.
#[derive(Debug, Clone, Serialize)]
pub struct ErrorCodeDoc {
    pub code: &'static str,
    pub category: &'static str,
    pub description: &'static str,
    pub recovery: &'static str,
}

/// Ranked command match returned by `doudou cli search`.
#[derive(Debug, Clone, Serialize)]
pub struct SearchMatch {
    pub path: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub aliases: Vec<String>,
    pub about: String,
    pub semantics: String,
    pub two_phase_write: bool,
    pub score: u32,
    pub matched_in: Vec<String>,
    pub key_flags: Vec<String>,
}

/// Full structured output returned by `doudou cli search --json`.
#[derive(Debug, Clone, Serialize)]
pub struct SearchResponse {
    pub query: String,
    pub total_matches: usize,
    pub matches: Vec<SearchMatch>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub matched_error_codes: Vec<ErrorCodeDoc>,
}

#[derive(Debug, Clone, Copy)]
struct CommandContract {
    semantics: &'static str,
    broker_command: Option<&'static str>,
    two_phase_write: bool,
    error_codes: &'static [&'static str],
    keywords: &'static [&'static str],
}

fn command_contract(path: &str) -> CommandContract {
    match path {
        "doctor" => CommandContract {
            semantics: "local",
            broker_command: None,
            two_phase_write: false,
            error_codes: &[
                "LEGACY_UNSUPPORTED",
                "CHROME_NOT_FOUND",
                "CHROME_LAUNCH_FAILED",
                "CDP_CONNECT_FAILED",
                "PROFILE_NOT_FOUND",
            ],
            keywords: &[
                "health",
                "chrome",
                "appdata",
                "doudou_home",
                "cdp",
                "probe",
                "readiness",
            ],
        },
        "profile list" => CommandContract {
            semantics: "local",
            broker_command: None,
            two_phase_write: false,
            error_codes: &["LEGACY_UNSUPPORTED", "IO_ERROR"],
            keywords: &["appdata", "profiles", "accounts", "lock", "doudou_home"],
        },
        "profile create" => CommandContract {
            semantics: "local",
            broker_command: None,
            two_phase_write: false,
            error_codes: &[
                "PROFILE_EXISTS",
                "LEGACY_UNSUPPORTED",
                "INVALID_PATH_SEGMENT",
            ],
            keywords: &["appdata", "new", "chrome-config", "v3", "doudou_home"],
        },
        "profile status" => CommandContract {
            semantics: "local",
            broker_command: Some("status"),
            two_phase_write: false,
            error_codes: &[
                "PROFILE_NOT_FOUND",
                "PROFILE_METADATA_MISMATCH",
                "LEGACY_UNSUPPORTED",
            ],
            keywords: &["show", "info", "appdata", "broker", "paths"],
        },
        "profile login" | "session login" => CommandContract {
            semantics: "control",
            broker_command: Some("login.wait"),
            two_phase_write: false,
            error_codes: &[
                "PROFILE_NOT_FOUND",
                "AUTH_LOGIN_REQUIRED",
                "LOGIN_REQUIRED",
                "VERIFICATION_REQUIRED",
                "BROKER_START_TIMEOUT",
            ],
            keywords: &["qrcode", "auth", "headed", "takeover", "signin"],
        },
        "profile delete" => CommandContract {
            semantics: "local",
            broker_command: None,
            two_phase_write: false,
            error_codes: &["PROFILE_NOT_FOUND", "PROFILE_LOCKED", "LEGACY_UNSUPPORTED"],
            keywords: &["remove", "appdata", "cleanup", "messages"],
        },
        "session status" => CommandContract {
            semantics: "read",
            broker_command: Some("session.status"),
            two_phase_write: false,
            error_codes: &[
                "PROFILE_NOT_FOUND",
                "AUTH_LOGIN_REQUIRED",
                "VERIFICATION_REQUIRED",
                "BROKER_NOT_RUNNING",
            ],
            keywords: &["login", "takeover", "headless", "im", "auth"],
        },
        "session refresh" => CommandContract {
            semantics: "control",
            broker_command: Some("session.refresh"),
            two_phase_write: false,
            error_codes: &[
                "AUTH_LOGIN_REQUIRED",
                "LOGIN_REQUIRED",
                "VERIFICATION_REQUIRED",
            ],
            keywords: &["im", "init", "index", "reconnect"],
        },
        "session auth start" => CommandContract {
            semantics: "control",
            broker_command: Some("session.auth.start"),
            two_phase_write: false,
            error_codes: &[
                "PROFILE_NOT_FOUND",
                "AUTH_QR_NOT_READY",
                "VERIFICATION_REQUIRED",
            ],
            keywords: &["headless", "qr", "qrcode", "scan", "login", "sms", "auth"],
        },
        "session auth status" => CommandContract {
            semantics: "read",
            broker_command: Some("session.auth.status"),
            two_phase_write: false,
            error_codes: &["PROFILE_NOT_FOUND", "BROKER_NOT_RUNNING"],
            keywords: &["headless", "qr", "qrcode", "scan", "sms", "login", "status"],
        },
        "session auth refresh-qr" => CommandContract {
            semantics: "control",
            broker_command: Some("session.auth.refresh_qr"),
            two_phase_write: false,
            error_codes: &["PROFILE_NOT_FOUND", "AUTH_QR_NOT_READY"],
            keywords: &["headless", "qr", "qrcode", "refresh", "expired", "login"],
        },
        "session auth sms-send" => CommandContract {
            semantics: "control",
            broker_command: Some("session.auth.sms_send"),
            two_phase_write: false,
            error_codes: &[
                "PROFILE_NOT_FOUND",
                "AUTH_SMS_FORM_NOT_FOUND",
                "AUTH_SMS_BUTTON_NOT_FOUND",
                "AUTH_SMS_SEND_FAILED",
                "VERIFICATION_REQUIRED",
            ],
            keywords: &["headless", "sms", "phone", "otp", "code", "login"],
        },
        "session auth sms-verify" => CommandContract {
            semantics: "control",
            broker_command: Some("session.auth.sms_verify"),
            two_phase_write: false,
            error_codes: &[
                "MISSING_ARGUMENT",
                "AUTH_SMS_FORM_NOT_FOUND",
                "AUTH_SMS_VERIFY_FAILED",
                "VERIFICATION_REQUIRED",
            ],
            keywords: &["headless", "sms", "otp", "code", "verify", "login"],
        },
        "session auth wait" => CommandContract {
            semantics: "control",
            broker_command: Some("session.auth.wait"),
            two_phase_write: false,
            error_codes: &[
                "AUTH_LOGIN_TIMEOUT",
                "VERIFICATION_REQUIRED",
                "PROFILE_NOT_FOUND",
            ],
            keywords: &["headless", "qr", "qrcode", "scan", "wait", "login"],
        },
        "session close" => CommandContract {
            semantics: "control",
            broker_command: Some("shutdown"),
            two_phase_write: false,
            error_codes: &["PROFILE_NOT_FOUND", "BROKER_STOP_TIMEOUT"],
            keywords: &["logout", "stop", "shutdown", "chrome"],
        },
        "broker list" => CommandContract {
            semantics: "local",
            broker_command: None,
            two_phase_write: false,
            error_codes: &["IO_ERROR"],
            keywords: &["daemon", "running", "appdata", "processes"],
        },
        "broker status" => CommandContract {
            semantics: "local",
            broker_command: Some("status"),
            two_phase_write: false,
            error_codes: &["PROFILE_NOT_FOUND", "BROKER_NOT_RUNNING"],
            keywords: &["daemon", "health", "pid"],
        },
        "broker ping" => CommandContract {
            semantics: "read",
            broker_command: Some("ping"),
            two_phase_write: false,
            error_codes: &["BROKER_START_FAILED", "BROKER_CONNECT_FAILED"],
            keywords: &["pong", "liveness", "daemon"],
        },
        "broker stop" => CommandContract {
            semantics: "control",
            broker_command: Some("shutdown"),
            two_phase_write: false,
            error_codes: &["BROKER_STOP_TIMEOUT", "PROFILE_NOT_FOUND"],
            keywords: &["shutdown", "kill", "daemon"],
        },
        "broker stop-all" => CommandContract {
            semantics: "control",
            broker_command: Some("shutdown"),
            two_phase_write: false,
            error_codes: &["IO_ERROR"],
            keywords: &["shutdown", "all", "cleanup"],
        },
        "broker test-start" => CommandContract {
            semantics: "control",
            broker_command: Some("status"),
            two_phase_write: false,
            error_codes: &["BROKER_START_FAILED", "BROKER_START_TIMEOUT"],
            keywords: &["fake-browser", "smoke", "start"],
        },
        "broker serve" => CommandContract {
            semantics: "control",
            broker_command: None,
            two_phase_write: false,
            error_codes: &["PROFILE_LOCKED", "BROKER_BIND_FAILED", "PROFILE_NOT_FOUND"],
            keywords: &["daemon", "tcp", "server", "loopback"],
        },
        "conversations refresh" => CommandContract {
            semantics: "read",
            broker_command: Some("conversations.refresh"),
            two_phase_write: false,
            error_codes: &[
                "AUTH_LOGIN_REQUIRED",
                "PROTOCOL_SCHEMA_DRIFTED",
                "RATE_LIMITED",
            ],
            keywords: &[
                "chat", "group", "direct", "stranger", "request", "inbox", "list",
            ],
        },
        "conversations resolve" => CommandContract {
            semantics: "read",
            broker_command: Some("conversations.refresh"),
            two_phase_write: false,
            error_codes: &["RECIPIENT_NOT_FOUND", "CONVERSATION_AMBIGUOUS"],
            keywords: &["chat", "find", "lookup", "stranger"],
        },
        "conversations select" => CommandContract {
            semantics: "read",
            broker_command: Some("conversations.refresh"),
            two_phase_write: false,
            error_codes: &[
                "CONVERSATION_SNAPSHOT_DRIFTED",
                "INVALID_CONVERSATION_RANGE",
                "CONVERSATION_NUMBER_OUT_OF_RANGE",
            ],
            keywords: &["chat", "pick", "range", "index"],
        },
        "messages collect" => CommandContract {
            semantics: "read",
            broker_command: Some("messages.collect"),
            two_phase_write: false,
            error_codes: &[
                "RECIPIENT_NOT_FOUND",
                "CONVERSATION_AMBIGUOUS",
                "MEDIA_MD5_MISMATCH",
                "PROTOCOL_SCHEMA_DRIFTED",
                "RATE_LIMITED",
            ],
            keywords: &[
                "chat", "history", "stranger", "request", "images", "aes-gcm", "protobuf",
            ],
        },
        "protocol send" | "chat send" => CommandContract {
            semantics: "write",
            broker_command: Some("chat.send.preview / chat.send.execute"),
            two_phase_write: true,
            error_codes: &[
                "PLAN_ID_REQUIRED",
                "PLAN_NOT_FOUND",
                "PLAN_EXPIRED",
                "PLAN_DRIFTED",
                "PLAN_ALREADY_USED",
                "STRANGER_WRITE_UNSUPPORTED",
                "MENTION_UNSUPPORTED",
                "IMAGE_NOT_FOUND",
                "WRITE_RESULT_UNKNOWN",
                "PAGE_ADAPTER_DRIFTED",
            ],
            keywords: &[
                "message", "text", "image", "photo", "reply", "plan", "confirm",
            ],
        },
        "protocol send-batch" | "chat send-batch" => CommandContract {
            semantics: "write",
            broker_command: Some("chat.send.batch.preview / chat.send.batch.execute"),
            two_phase_write: true,
            error_codes: &[
                "PLAN_ID_REQUIRED",
                "PLAN_NOT_FOUND",
                "PLAN_EXPIRED",
                "PLAN_DRIFTED",
                "PLAN_ALREADY_USED",
                "STRANGER_WRITE_UNSUPPORTED",
                "WRITE_RESULT_UNKNOWN",
            ],
            keywords: &[
                "batch",
                "broadcast",
                "bulk",
                "multiple",
                "send",
                "plan",
                "confirm",
            ],
        },
        "protocol reply" | "chat reply" => CommandContract {
            semantics: "write",
            broker_command: Some("chat.send.preview / chat.send.execute"),
            two_phase_write: true,
            error_codes: &[
                "PLAN_ID_REQUIRED",
                "PLAN_ALREADY_USED",
                "REPLY_TARGET_NOT_VISIBLE",
                "STRANGER_WRITE_UNSUPPORTED",
                "WRITE_RESULT_UNKNOWN",
            ],
            keywords: &["quote", "respond", "message", "send", "plan", "confirm"],
        },
        "protocol capture" => CommandContract {
            semantics: "control",
            broker_command: Some("protocol.capture"),
            two_phase_write: false,
            error_codes: &["CAPTURE_INCOMPLETE", "AUTH_LOGIN_REQUIRED"],
            keywords: &["protobuf", "network", "debug", "diagnose", "capture"],
        },
        "chat like" => CommandContract {
            semantics: "write",
            broker_command: Some("chat.like.preview / chat.like.execute"),
            two_phase_write: true,
            error_codes: &[
                "PLAN_ID_REQUIRED",
                "PLAN_ALREADY_USED",
                "STRANGER_WRITE_UNSUPPORTED",
                "MESSAGE_TARGET_NOT_VISIBLE",
                "SELF_IDENTITY_UNRESOLVED",
                "WRITE_RESULT_UNKNOWN",
            ],
            keywords: &["reaction", "love", "heart", "message", "plan", "confirm"],
        },
        "identity resolve" => CommandContract {
            semantics: "read",
            broker_command: Some("identity.resolve"),
            two_phase_write: false,
            error_codes: &[
                "LEGACY_UNSUPPORTED",
                "IDENTITY_NOT_FOUND",
                "PROTOCOL_SCHEMA_DRIFTED",
            ],
            keywords: &["user", "douyin-id", "unique_id", "sec_uid", "uid"],
        },
        "videos collect" => CommandContract {
            semantics: "read",
            broker_command: Some("videos.collect"),
            two_phase_write: false,
            error_codes: &[
                "IDENTITY_NOT_FOUND",
                "PROTOCOL_SCHEMA_DRIFTED",
                "RATE_LIMITED",
                "VERIFICATION_REQUIRED",
            ],
            keywords: &["user", "author", "aweme", "feed", "posts"],
        },
        "video collect" => CommandContract {
            semantics: "read",
            broker_command: Some("video.collect"),
            two_phase_write: false,
            error_codes: &[
                "VIDEO_TARGET_REQUIRED",
                "PROTOCOL_SCHEMA_DRIFTED",
                "RATE_LIMITED",
            ],
            keywords: &["aweme", "comments", "replies", "detail", "cover"],
        },
        "video comment" => CommandContract {
            semantics: "write",
            broker_command: Some("video.interaction.preview / video.interaction.execute"),
            two_phase_write: true,
            error_codes: &[
                "VIDEO_TARGET_REQUIRED",
                "COMMENT_TEXT_REQUIRED",
                "PLAN_ID_REQUIRED",
                "PLAN_ALREADY_USED",
                "WRITE_RESULT_UNKNOWN",
            ],
            keywords: &["post", "write", "plan", "confirm"],
        },
        "video reply" => CommandContract {
            semantics: "write",
            broker_command: Some("video.interaction.preview / video.interaction.execute"),
            two_phase_write: true,
            error_codes: &[
                "VIDEO_TARGET_REQUIRED",
                "COMMENT_TEXT_REQUIRED",
                "COMMENT_TARGET_NOT_FOUND",
                "COMMENT_TARGET_DRIFTED",
                "COMMENT_TARGET_NOT_VISIBLE",
                "PLAN_ID_REQUIRED",
                "PLAN_ALREADY_USED",
                "WRITE_RESULT_UNKNOWN",
            ],
            keywords: &["comment", "respond", "plan", "confirm"],
        },
        "video like" => CommandContract {
            semantics: "write",
            broker_command: Some("video.interaction.preview / video.interaction.execute"),
            two_phase_write: true,
            error_codes: &[
                "VIDEO_TARGET_REQUIRED",
                "COMMENT_TARGET_NOT_FOUND",
                "COMMENT_TARGET_DRIFTED",
                "COMMENT_TARGET_NOT_VISIBLE",
                "PLAN_ID_REQUIRED",
                "PLAN_ALREADY_USED",
                "WRITE_RESULT_UNKNOWN",
            ],
            keywords: &["comment", "digg", "heart", "plan", "confirm"],
        },
        "page-doctor" | "chat diagnose" => CommandContract {
            semantics: "read",
            broker_command: Some("page.doctor"),
            two_phase_write: false,
            error_codes: &[
                "PAGE_ADAPTER_DRIFTED",
                "PAGE_DOCTOR_STAGE_TIMEOUT",
                "AUTH_LOGIN_REQUIRED",
            ],
            keywords: &[
                "ui-probe",
                "diagnose",
                "selector",
                "dom",
                "drift",
                "components",
            ],
        },
        "cli search" | "cli schema" => CommandContract {
            semantics: "meta",
            broker_command: None,
            two_phase_write: false,
            error_codes: &["UNKNOWN_COMMAND"],
            keywords: &["discover", "help", "introspect", "json", "flags", "errors"],
        },
        _ => CommandContract {
            semantics: "group",
            broker_command: None,
            two_phase_write: false,
            error_codes: &[],
            keywords: &[],
        },
    }
}

/// Complete catalog of stable machine-readable Doudou error codes.
pub fn error_code_catalog() -> Vec<ErrorCodeDoc> {
    vec![
        ErrorCodeDoc {
            code: "AUTH_QR_NOT_READY",
            category: "AUTH",
            description: "The headless Douyin login page did not render a QR code or SMS form within the wait budget.",
            recovery: "Retry `doudou session auth start --profile <name> --reload` or fall back to headed `doudou session login`.",
        },
        ErrorCodeDoc {
            code: "AUTH_SMS_SEND_FAILED",
            category: "AUTH",
            description: "Douyin rejected the SMS verification code request (e.g. invalid phone or rate limit).",
            recovery: "Inspect the error message, verify the phone number, or use QR code login instead.",
        },
        ErrorCodeDoc {
            code: "AUTH_SMS_VERIFY_FAILED",
            category: "AUTH",
            description: "Douyin rejected the submitted SMS code or timed out confirming login.",
            recovery: "Request a fresh SMS code with `doudou session auth sms-send` or switch to QR login.",
        },
        ErrorCodeDoc {
            code: "AUTH_LOGIN_TIMEOUT",
            category: "AUTH",
            description: "Headless QR/SMS authentication did not complete within the configured timeout.",
            recovery: "Refresh the QR code with `doudou session auth refresh-qr` or re-run `doudou session auth wait`.",
        },
        ErrorCodeDoc {
            code: "PROFILE_EXISTS",
            category: "PROFILE",
            description: "A profile with the same encoded profile_id already exists under doudou_home.",
            recovery: "Reuse the existing profile from `doudou profile list` or choose another name.",
        },
        ErrorCodeDoc {
            code: "PROFILE_NOT_FOUND",
            category: "PROFILE",
            description: "The requested profile does not exist under <doudou_home>/profiles/.",
            recovery: "Run `doudou profile list` or create it with `doudou profile create --profile <name>`.",
        },
        ErrorCodeDoc {
            code: "PROFILE_LOCKED",
            category: "PROFILE",
            description: "Another live broker or browser process currently holds the profile lock.",
            recovery: "Wait for the active command to complete or inspect via `doudou broker list`.",
        },
        ErrorCodeDoc {
            code: "PROFILE_METADATA_MISMATCH",
            category: "PROFILE",
            description: "profile.json profile_id does not match its directory name.",
            recovery: "Inspect profile.json under <doudou_home>/profiles/<name>/ before continuing.",
        },
        ErrorCodeDoc {
            code: "LEGACY_UNSUPPORTED",
            category: "PROFILE",
            description: "Legacy Profile/ casing, pre-v3 Camoufox profile metadata, or unversioned identity records were detected.",
            recovery: "Use a clean data root or fresh `doudou.profile.v3` profile; legacy files are never mutated automatically.",
        },
        ErrorCodeDoc {
            code: "PLAN_ID_REQUIRED",
            category: "PLAN",
            description: "`--confirm` was passed without `--plan-id <id>`.",
            recovery: "Run the command without `--confirm` first, show the preview to the user, then pass `--confirm --plan-id <id>`.",
        },
        ErrorCodeDoc {
            code: "PLAN_NOT_FOUND",
            category: "PLAN",
            description: "The specified write plan file does not exist under message/profiles/<name>/plans/.",
            recovery: "Create a fresh preview plan before confirming.",
        },
        ErrorCodeDoc {
            code: "PLAN_EXPIRED",
            category: "PLAN",
            description: "The write plan exceeded its TTL before confirmation.",
            recovery: "Generate a fresh preview plan and ask the user to confirm again.",
        },
        ErrorCodeDoc {
            code: "PLAN_DRIFTED",
            category: "PLAN",
            description: "The confirmed payload hash does not match the stored plan payload.",
            recovery: "Never reuse a plan for different parameters; generate a new preview plan.",
        },
        ErrorCodeDoc {
            code: "PLAN_ALREADY_USED",
            category: "PLAN",
            description: "The single-use write plan is already executing or in a terminal state (sent/failed/unknown).",
            recovery: "Do NOT retry automatically; verify external state via read-only commands first.",
        },
        ErrorCodeDoc {
            code: "PLAN_OPERATION_MISMATCH",
            category: "PLAN",
            description: "The plan_id belongs to a different write operation than the invoked command.",
            recovery: "Run the matching command for the plan's operation or create a new plan.",
        },
        ErrorCodeDoc {
            code: "WRITE_RESULT_UNKNOWN",
            category: "WRITE",
            description: "An external write may have been submitted before a disconnect, timeout, or read-back mismatch.",
            recovery: "STOP and verify with read-only `messages collect` or `video collect`. Never auto-retry.",
        },
        ErrorCodeDoc {
            code: "STRANGER_WRITE_UNSUPPORTED",
            category: "WRITE",
            description: "Attempted to send, reply, or like inside a read-only stranger message request.",
            recovery: "Stranger message requests are read-only; only established conversations support writes.",
        },
        ErrorCodeDoc {
            code: "MENTION_UNSUPPORTED",
            category: "WRITE",
            description: "Group `@` mentions (`--at`) are not supported and fail closed before broker startup.",
            recovery: "Remove `--at` and send plain text or a targeted reply.",
        },
        ErrorCodeDoc {
            code: "AUTH_LOGIN_REQUIRED",
            category: "AUTH",
            description: "The Chrome profile is not logged into Douyin.",
            recovery: "Run `doudou session login --profile <name>` for interactive headed login.",
        },
        ErrorCodeDoc {
            code: "LOGIN_REQUIRED",
            category: "AUTH",
            description: "Douyin rejected the operation because authentication cookies expired or are missing.",
            recovery: "Run `doudou session login --profile <name>` and re-check `doudou session status`.",
        },
        ErrorCodeDoc {
            code: "VERIFICATION_REQUIRED",
            category: "AUTH",
            description: "Douyin presented a captcha/SMS/risk challenge blocking automated execution.",
            recovery: "Complete the challenge during headed takeover or inspect `doudou session status`.",
        },
        ErrorCodeDoc {
            code: "PROTOCOL_SCHEMA_DRIFTED",
            category: "PROTOCOL",
            description: "Douyin Web or IM protocol response shape diverged from the verified contract.",
            recovery: "Run `doudou page-doctor` or `doudou protocol capture` to collect fresh evidence.",
        },
        ErrorCodeDoc {
            code: "RATE_LIMITED",
            category: "PROTOCOL",
            description: "Douyin returned HTTP 429 or throttling status.",
            recovery: "Back off and retry read-only collection later with smaller `--max-pages`.",
        },
        ErrorCodeDoc {
            code: "MEDIA_MD5_MISMATCH",
            category: "PROTOCOL",
            description: "Downloaded or AES-GCM decrypted image failed MD5 integrity verification.",
            recovery: "Re-collect the conversation or inspect the saved media metadata.",
        },
        ErrorCodeDoc {
            code: "PAGE_ADAPTER_DRIFTED",
            category: "UI",
            description: "Live Douyin DOM selectors or element geometry no longer match the UI component contract.",
            recovery: "Run `doudou page-doctor --profile <name>` to inspect component drift findings.",
        },
        ErrorCodeDoc {
            code: "MESSAGE_TARGET_NOT_VISIBLE",
            category: "UI",
            description: "The target chat message could not be unambiguously located in the visible DOM.",
            recovery: "Ensure the conversation is collected and the message is reachable in recent chat history.",
        },
        ErrorCodeDoc {
            code: "COMMENT_TARGET_NOT_VISIBLE",
            category: "UI",
            description: "The target video comment could not be unambiguously located in the visible DOM.",
            recovery: "Re-collect video comments and verify the comment still exists.",
        },
        ErrorCodeDoc {
            code: "UI_OCCLUDED",
            category: "UI",
            description: "A blocking modal, header, or overlay occluded the verified click target.",
            recovery: "Inspect with `doudou page-doctor` before retrying.",
        },
        ErrorCodeDoc {
            code: "BROKER_NOT_RUNNING",
            category: "BROKER",
            description: "No live broker process is currently running for the specified profile.",
            recovery: "Any business command will auto-start the broker, or run `doudou broker ping --profile <name>`.",
        },
        ErrorCodeDoc {
            code: "BROKER_START_FAILED",
            category: "BROKER",
            description: "Failed to spawn the detached `doudou broker serve` process.",
            recovery: "Run `doudou doctor --launch-probe` and inspect `broker.stderr.log` under the profile runtime dir.",
        },
        ErrorCodeDoc {
            code: "BROKER_START_TIMEOUT",
            category: "BROKER",
            description: "Spawned broker did not publish its descriptor within the startup timeout.",
            recovery: "Check `stderr_tail` in the error details and verify Chrome health via `doudou doctor --launch-probe`.",
        },
        ErrorCodeDoc {
            code: "BROKER_STOP_TIMEOUT",
            category: "BROKER",
            description: "Broker did not release its OS process and lock within the shutdown timeout.",
            recovery: "Check `doudou broker list` and stop lingering processes.",
        },
        ErrorCodeDoc {
            code: "CHROME_NOT_FOUND",
            category: "BROWSER",
            description: "No Chrome, Chromium, or Edge binary was discovered on the host.",
            recovery: "Install Google Chrome or set `DOUDOU_CHROME_BIN` / `--chrome-binary`.",
        },
    ]
}

fn introspect_arg(arg: &Arg) -> Option<ArgSchema> {
    if arg.is_hide_set() {
        return None;
    }
    let id = arg.get_id().as_str();
    if matches!(id, "help" | "version") {
        return None;
    }
    let takes_value = arg.get_action().takes_values();
    let multiple = matches!(arg.get_action(), ArgAction::Append)
        || arg
            .get_num_args()
            .is_some_and(|range| range.max_values() > 1);
    let default_values: Vec<String> = arg
        .get_default_values()
        .iter()
        .map(|val| val.to_string_lossy().into_owned())
        .collect();
    let possible_values: Vec<String> = arg
        .get_possible_values()
        .into_iter()
        .filter(|pv| !pv.is_hide_set())
        .map(|pv| pv.get_name().to_owned())
        .collect();
    let value_name = if takes_value {
        arg.get_value_names()
            .and_then(|names| names.first())
            .map(|name| name.as_str().to_owned())
    } else {
        None
    };

    Some(ArgSchema {
        id: id.to_owned(),
        long: arg.get_long().map(ToOwned::to_owned),
        short: arg.get_short(),
        positional: arg.is_positional(),
        required: arg.is_required_set(),
        takes_value,
        multiple,
        value_name,
        default_values,
        possible_values,
        help: arg.get_help().map(ToString::to_string),
        long_help: arg.get_long_help().map(ToString::to_string),
    })
}

fn introspect_command(cmd: &Command, prefix: &[String], skip_globals: bool) -> CommandSchema {
    let mut path_parts = prefix.to_vec();
    if !prefix.is_empty() || cmd.get_name() != "doudou" {
        path_parts.push(cmd.get_name().to_owned());
    }
    let path = path_parts.join(" ");
    let contract = command_contract(&path);

    let args: Vec<ArgSchema> = cmd
        .get_arguments()
        .filter(|arg| !skip_globals || !arg.is_global_set())
        .filter_map(introspect_arg)
        .collect();

    let subcommands: Vec<CommandSchema> = cmd
        .get_subcommands()
        .filter(|sub| !sub.is_hide_set() && sub.get_name() != "help")
        .map(|sub| introspect_command(sub, &path_parts, true))
        .collect();

    let aliases: Vec<String> = cmd.get_visible_aliases().map(ToOwned::to_owned).collect();

    CommandSchema {
        path,
        name: cmd.get_name().to_owned(),
        aliases,
        about: cmd.get_about().map(ToString::to_string),
        long_about: cmd.get_long_about().map(ToString::to_string),
        examples: cmd.get_after_help().map(ToString::to_string),
        semantics: contract.semantics.to_owned(),
        broker_command: contract.broker_command.map(ToOwned::to_owned),
        two_phase_write: contract.two_phase_write,
        related_error_codes: contract
            .error_codes
            .iter()
            .map(|code| (*code).to_owned())
            .collect(),
        args,
        subcommands,
    }
}

fn find_subcommand<'a>(root: &'a Command, tokens: &[String]) -> Option<(Vec<String>, &'a Command)> {
    let mut current = root;
    let mut resolved = Vec::new();
    for raw in tokens {
        let needle = raw.trim().to_lowercase();
        if needle.is_empty() || needle == "doudou" {
            continue;
        }
        let next = current.get_subcommands().find(|sub| {
            sub.get_name().eq_ignore_ascii_case(&needle)
                || sub
                    .get_all_aliases()
                    .any(|alias| alias.eq_ignore_ascii_case(&needle))
        })?;
        resolved.push(next.get_name().to_owned());
        current = next;
    }
    Some((resolved, current))
}

/// Build machine-readable JSON schema for the entire CLI or a specific subcommand path.
pub fn build_schema(command_path: &[String]) -> Result<Value> {
    let root = Cli::command();
    let global_flags: Vec<ArgSchema> = root
        .get_arguments()
        .filter(|arg| arg.is_global_set())
        .filter_map(introspect_arg)
        .collect();

    let errors = error_code_catalog();
    let mut errors_by_category: BTreeMap<&str, Vec<&ErrorCodeDoc>> = BTreeMap::new();
    for doc in &errors {
        errors_by_category
            .entry(doc.category)
            .or_default()
            .push(doc);
    }

    let contracts = json!({
        "storage_boundaries": {
            "doudou_home": {
                "override_flag": "--home <DIR>",
                "env_var": "DOUDOU_HOME",
                "defaults": {
                    "windows": "%LOCALAPPDATA%\\doudou",
                    "macos": "~/Library/Application Support/doudou",
                    "linux": "${XDG_DATA_HOME:-~/.local/share}/doudou"
                },
                "stores": ["profiles/<profile_id>/profile.json (doudou.profile.v3)", "profiles/<profile_id>/chrome-config.json", "profiles/<profile_id>/browser-data/", "brokers/<hash>.json"]
            },
            "data_root": {
                "override_flag": "--data-root <DIR>",
                "env_var": "DOUDOU_DATA_ROOT",
                "default": "doudou_home (OS AppData)",
                "stores": ["message/profiles/<profile_id>/conversations/", "message/profiles/<profile_id>/plans/", "message/profiles/<profile_id>/sends/", "message/profiles/<profile_id>/interactions/", "message/profiles/<profile_id>/diagnostics/"]
            }
        },
        "ndjson_contract": {
            "runtime_commands_stdout": "compact NDJSON (one JSON object per line)",
            "events": [
                {"event": "started", "fields": ["command", "home", "data_root", "version"]},
                {"event": "result", "fields": ["result"]},
                {"event": "error", "fields": ["code", "message", "details?"]}
            ],
            "meta_commands_stdout": "`cli schema` and `cli search --json` emit direct JSON; `cli search` emits ranked plain text."
        },
        "write_safety_contract": {
            "phase_1_preview": "Run any write command without --confirm to create a single-use plan (status=confirmation_required) and display the preview to the user.",
            "phase_2_execute": "After explicit user confirmation, re-run with --confirm --plan-id <id>. Plan atomically transitions confirmation_required -> executing -> sent|failed|unknown.",
            "unknown_handling": "On WRITE_RESULT_UNKNOWN, never auto-retry; verify via read-only `messages collect` or `video collect` first."
        }
    });

    if command_path.is_empty() {
        let commands: Vec<CommandSchema> = root
            .get_subcommands()
            .filter(|sub| !sub.is_hide_set() && sub.get_name() != "help")
            .map(|sub| introspect_command(sub, &[], true))
            .collect();
        return Ok(json!({
            "schema": CLI_SCHEMA_VERSION,
            "name": root.get_name(),
            "version": env!("CARGO_PKG_VERSION"),
            "about": root.get_about().map(ToString::to_string),
            "long_about": root.get_long_about().map(ToString::to_string),
            "global_flags": global_flags,
            "contracts": contracts,
            "error_codes": errors,
            "error_categories": errors_by_category.keys().copied().collect::<Vec<_>>(),
            "commands": commands,
        }));
    }

    let Some((resolved_path, target)) = find_subcommand(&root, command_path) else {
        return Err(DoudouError::new(
            "UNKNOWN_COMMAND",
            format!(
                "Unknown command path for schema: `{}`. Run `doudou cli search <query>` to discover commands.",
                command_path.join(" ")
            ),
        ));
    };

    let prefix = if resolved_path.len() > 1 {
        &resolved_path[..resolved_path.len() - 1]
    } else {
        &[]
    };
    let schema = introspect_command(target, prefix, true);
    let scoped_errors: Vec<&ErrorCodeDoc> = errors
        .iter()
        .filter(|doc| {
            schema
                .related_error_codes
                .iter()
                .any(|code| code == doc.code)
        })
        .collect();

    Ok(json!({
        "schema": CLI_SCHEMA_VERSION,
        "name": root.get_name(),
        "version": env!("CARGO_PKG_VERSION"),
        "target_command": schema.path,
        "global_flags": global_flags,
        "command": schema,
        "related_error_codes": scoped_errors,
        "ndjson_contract": contracts["ndjson_contract"],
        "write_safety_contract": contracts["write_safety_contract"],
    }))
}

fn flatten_leaf_commands(schema: &CommandSchema, out: &mut Vec<CommandSchema>) {
    if schema.subcommands.is_empty() {
        out.push(schema.clone());
    } else {
        for sub in &schema.subcommands {
            flatten_leaf_commands(sub, out);
        }
    }
}

fn tokenize(input: &str) -> Vec<String> {
    input
        .split(|c: char| !c.is_alphanumeric() && c != '_')
        .map(str::to_lowercase)
        .filter(|token| !token.is_empty())
        .collect()
}

fn score_command(cmd: &CommandSchema, raw_query: &str, tokens: &[String]) -> Option<SearchMatch> {
    let contract = command_contract(&cmd.path);
    let path_lc = cmd.path.to_lowercase();
    let aliases_lc: Vec<String> = cmd.aliases.iter().map(|a| a.to_lowercase()).collect();
    let about_lc = cmd.about.as_deref().unwrap_or("").to_lowercase();
    let long_about_lc = cmd.long_about.as_deref().unwrap_or("").to_lowercase();
    let examples_lc = cmd.examples.as_deref().unwrap_or("").to_lowercase();
    let flags_lc: Vec<String> = cmd
        .args
        .iter()
        .map(|arg| {
            let mut parts = Vec::new();
            if let Some(long) = &arg.long {
                parts.push(format!("--{long}"));
            }
            parts.push(arg.id.clone());
            if let Some(help) = &arg.help {
                parts.push(help.clone());
            }
            if let Some(long_help) = &arg.long_help {
                parts.push(long_help.clone());
            }
            parts.extend(arg.possible_values.clone());
            parts.join(" ").to_lowercase()
        })
        .collect();
    let errors_lc: Vec<String> = cmd
        .related_error_codes
        .iter()
        .map(|code| code.to_lowercase())
        .collect();
    let keywords_lc: Vec<String> = contract
        .keywords
        .iter()
        .map(|kw| kw.to_lowercase())
        .collect();

    let mut score = 0u32;
    let mut matched_fields = Vec::new();
    let mut matched_tokens = 0usize;

    let query_lc = raw_query.trim().to_lowercase();
    if path_lc == query_lc || aliases_lc.iter().any(|a| a == &query_lc) {
        score += 120;
        matched_fields.push("exact_command".to_owned());
    } else if !query_lc.is_empty() && path_lc.contains(&query_lc) {
        score += 70;
        matched_fields.push("command_path".to_owned());
    }

    for token in tokens {
        let mut token_hit = false;
        if path_lc.split_whitespace().any(|seg| seg == token)
            || aliases_lc.iter().any(|a| a == token)
        {
            score += 55;
            token_hit = true;
            push_unique(&mut matched_fields, "command");
        } else if path_lc.contains(token) || aliases_lc.iter().any(|a| a.contains(token)) {
            score += 35;
            token_hit = true;
            push_unique(&mut matched_fields, "command");
        }
        if about_lc.contains(token) {
            score += 24;
            token_hit = true;
            push_unique(&mut matched_fields, "summary");
        }
        if long_about_lc.contains(token) {
            score += 14;
            token_hit = true;
            push_unique(&mut matched_fields, "description");
        }
        if flags_lc.iter().any(|f| f.contains(token)) {
            score += 20;
            token_hit = true;
            push_unique(&mut matched_fields, "flags");
        }
        if keywords_lc.iter().any(|k| k.contains(token)) {
            score += 22;
            token_hit = true;
            push_unique(&mut matched_fields, "intent");
        }
        if errors_lc.iter().any(|e| e.contains(token)) {
            score += 18;
            token_hit = true;
            push_unique(&mut matched_fields, "error_codes");
        }
        if examples_lc.contains(token) {
            score += 8;
            token_hit = true;
            push_unique(&mut matched_fields, "examples");
        }
        if token_hit {
            matched_tokens += 1;
        }
    }

    if score == 0 {
        return None;
    }
    if !tokens.is_empty() && matched_tokens == tokens.len() {
        score += 30;
    }

    let key_flags: Vec<String> = cmd
        .args
        .iter()
        .filter_map(|arg| {
            arg.long
                .as_ref()
                .map(|long| format!("--{long}"))
                .or_else(|| {
                    arg.positional
                        .then(|| format!("<{}>", arg.id.to_uppercase()))
                })
        })
        .take(6)
        .collect();

    Some(SearchMatch {
        path: cmd.path.clone(),
        aliases: cmd.aliases.clone(),
        about: cmd.about.clone().unwrap_or_default(),
        semantics: cmd.semantics.clone(),
        two_phase_write: cmd.two_phase_write,
        score,
        matched_in: matched_fields,
        key_flags,
    })
}

fn push_unique(items: &mut Vec<String>, label: &str) {
    if !items.iter().any(|existing| existing == label) {
        items.push(label.to_owned());
    }
}

/// Search across the live `clap::Command` tree and error catalog, returning ranked results.
pub fn search_cli(query: &str, limit: usize) -> SearchResponse {
    let root = Cli::command();
    let tokens = tokenize(query);
    let mut leaves = Vec::new();
    for sub in root
        .get_subcommands()
        .filter(|sub| !sub.is_hide_set() && sub.get_name() != "help")
    {
        let schema = introspect_command(sub, &[], true);
        flatten_leaf_commands(&schema, &mut leaves);
    }

    let mut matches: Vec<SearchMatch> = leaves
        .iter()
        .filter_map(|cmd| score_command(cmd, query, &tokens))
        .collect();
    matches.sort_by(|left, right| {
        right
            .score
            .cmp(&left.score)
            .then_with(|| left.path.cmp(&right.path))
    });
    let total_matches = matches.len();
    matches.truncate(limit.max(1));

    let query_lc = query.trim().to_lowercase();
    let matched_error_codes: Vec<ErrorCodeDoc> = error_code_catalog()
        .into_iter()
        .filter(|doc| {
            let code_lc = doc.code.to_lowercase();
            let desc_lc = doc.description.to_lowercase();
            let rec_lc = doc.recovery.to_lowercase();
            (!query_lc.is_empty() && code_lc.contains(&query_lc))
                || (!tokens.is_empty()
                    && tokens.iter().all(|tok| {
                        code_lc.contains(tok)
                            || doc.category.to_lowercase().contains(tok)
                            || desc_lc.contains(tok)
                            || rec_lc.contains(tok)
                    }))
        })
        .take(8)
        .collect();

    SearchResponse {
        query: query.to_owned(),
        total_matches,
        matches,
        matched_error_codes,
    }
}

/// Format [`SearchResponse`] as compact human/agent-readable ranked text.
pub fn format_search_text(response: &SearchResponse) -> String {
    let mut out = String::new();
    if response.matches.is_empty() && response.matched_error_codes.is_empty() {
        let _ = writeln!(
            out,
            "No commands or error codes matched \"{}\". Run `doudou --help` or `doudou cli schema`.",
            response.query
        );
        return out;
    }

    let _ = writeln!(
        out,
        "Search results for \"{}\" ({} command match(es)):\n",
        response.query, response.total_matches
    );
    for item in &response.matches {
        let write_tag = if item.two_phase_write {
            " [2-phase write]"
        } else {
            ""
        };
        let alias_tag = if item.aliases.is_empty() {
            String::new()
        } else {
            format!(" (aliases: {})", item.aliases.join(", "))
        };
        let _ = writeln!(
            out,
            "  doudou {}{}{} [{}]",
            item.path, alias_tag, write_tag, item.semantics
        );
        if !item.about.is_empty() {
            let _ = writeln!(out, "    {}", item.about);
        }
        if !item.key_flags.is_empty() {
            let _ = writeln!(out, "    flags: {}", item.key_flags.join(" "));
        }
    }

    if !response.matched_error_codes.is_empty() {
        let _ = writeln!(out, "\nMatching error codes:");
        for err in &response.matched_error_codes {
            let _ = writeln!(
                out,
                "  {} [{}]: {} -> {}",
                err.code, err.category, err.description, err.recovery
            );
        }
    }

    let _ = writeln!(
        out,
        "\nNext: run `doudou cli schema <command>` or `doudou <command> -h` for full parameter contracts."
    );
    out
}
