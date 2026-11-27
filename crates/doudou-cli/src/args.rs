//! Command-line argument tree for `doudou` with rich self-describing help,
//! examples, aliases, and error-code guidance.

use std::path::PathBuf;

use clap::{Args, Parser, Subcommand, ValueEnum};

/// Root CLI definition for `doudou`.
#[derive(Debug, Parser)]
#[command(
    name = "doudou",
    version,
    about = "Self-describing Douyin Chrome CDP CLI for chat, video, profile, and broker automation.",
    long_about = "Doudou controls local Chrome profiles over native Rust CDP (never invoking Runtime.enable) to read Douyin group/direct chats, stranger message requests, images, users, videos, and comments, and to execute confirmed two-phase UI writes.\n\nStorage boundaries:\n  - Browser profiles & broker state live in OS AppData (%LOCALAPPDATA%\\doudou, ~/Library/Application Support/doudou, or ~/.local/share/doudou), overridable via --home or DOUDOU_HOME.\n  - Collected chats, media, write plans, receipts, and diagnostics live under <data-root>/message/profiles/<name>/, defaulting to <doudou_home> (OS AppData) and overridable via --data-root or DOUDOU_DATA_ROOT.\n\nOutput contract:\n  - All runtime commands emit compact NDJSON lines on stdout: started -> result (or error).\n  - Discovery commands (`cli search`, `cli schema`) output directly for fast agent and human inspection.",
    after_help = "QUICK START:\n  doudou doctor\n  doudou profile list\n  doudou profile create --profile work --display-name \"Work\"\n  doudou session login --profile work\n  doudou conversations refresh --profile work --kind all --inbox all\n  doudou cli search \"send\"\n  doudou cli schema protocol send",
    propagate_version = true
)]
pub struct Cli {
    /// Override user-level Doudou home directory (defaults to `DOUDOU_HOME` or OS `AppData`).
    #[arg(
        long,
        global = true,
        value_name = "DIR",
        help = "Override user-level Doudou AppData home (profiles/ and brokers/)",
        long_help = "Override the user-level Doudou home directory holding browser profiles (profiles/<name>/) and global broker registrations (brokers/). Priority: --home > DOUDOU_HOME > OS AppData (%LOCALAPPDATA%\\doudou, ~/Library/Application Support/doudou, ~/.local/share/doudou)."
    )]
    pub home: Option<PathBuf>,

    /// Override business data root (defaults to `DOUDOU_DATA_ROOT` or `<doudou_home>`).
    #[arg(
        long = "data-root",
        global = true,
        value_name = "DIR",
        help = "Override business data root (message/profiles/<name>/; defaults to --home / OS AppData)",
        long_help = "Override the business data root where message/profiles/<name>/ records, media, write plans, receipts, and diagnostics are stored. Priority: --data-root > DOUDOU_DATA_ROOT > <doudou_home> (OS AppData)."
    )]
    pub data_root: Option<PathBuf>,

    /// Subcommand to execute.
    #[command(subcommand)]
    pub command: Commands,
}

/// Top-level `doudou` commands.
#[derive(Debug, Subcommand)]
pub enum Commands {
    /// Inspect runtime readiness, OS AppData/data-root writability, Chrome discovery, profiles, and brokers.
    #[command(
        long_about = "Check environment health before operating on profiles. Verifies OS AppData (--home / DOUDOU_HOME) and --data-root (DOUDOU_DATA_ROOT, defaulting to <doudou_home>) writability, discovers local Chrome/Chromium/Edge binaries, lists profiles and active brokers, and optionally runs a live headless Chrome CDP launch probe without Runtime.enable.\n\nRejects legacy Profile/ directory casing with LEGACY_UNSUPPORTED before probing.",
        after_help = "EXAMPLES:\n  doudou doctor\n  doudou doctor --launch-probe\n  doudou doctor --profile work --chrome-binary \"C:\\Program Files\\Google\\Chrome\\Application\\chrome.exe\"\n\nERROR CODES: LEGACY_UNSUPPORTED, CHROME_NOT_FOUND, CHROME_LAUNCH_FAILED, CDP_CONNECT_FAILED, PROFILE_NOT_FOUND"
    )]
    Doctor(DoctorArgs),

    /// Create, list, inspect, log in, or delete Chrome profiles in OS `AppData`.
    #[command(
        long_about = "Manage isolated Douyin Chrome profiles stored under <doudou_home>/profiles/<name>/ (schema doudou.profile.v3, runtime chrome-rust, chrome-config.json). Collected records live under <data_root>/message/profiles/<name>/ (defaulting to <doudou_home>).",
        after_help = "EXAMPLES:\n  doudou profile list\n  doudou profile create --profile work --display-name \"Work Account\"\n  doudou profile status --profile work\n  doudou profile login --profile work --timeout 600\n  doudou profile delete --profile work --with-messages\n\nERROR CODES: PROFILE_EXISTS, PROFILE_NOT_FOUND, PROFILE_LOCKED, PROFILE_METADATA_MISMATCH, LEGACY_UNSUPPORTED"
    )]
    Profile {
        /// Profile operation to execute.
        #[command(subcommand)]
        command: ProfileCommand,
    },

    /// Check login/IM status, refresh IM initialization, run interactive login, or close a profile session.
    #[command(
        long_about = "Control the authenticated Douyin browser session for a profile. Supports checking browser/login/verification/IM-init status, refreshing IM initialization without re-login, launching interactive headed login takeover, or cleanly closing the session.",
        after_help = "EXAMPLES:\n  doudou session status --profile work\n  doudou session refresh --profile work\n  doudou session login --profile work --timeout 600\n  doudou session close --profile work\n\nERROR CODES: AUTH_LOGIN_REQUIRED, LOGIN_REQUIRED, VERIFICATION_REQUIRED, PROFILE_NOT_FOUND, BROKER_NOT_RUNNING"
    )]
    Session {
        /// Session operation to execute.
        #[command(subcommand)]
        command: SessionCommand,
    },

    /// Inspect, ping, start, stop, or serve per-profile loopback TCP broker processes.
    #[command(
        long_about = "Manage long-lived per-profile broker processes registered in <doudou_home>/brokers/. Normal read/write commands auto-start their broker via `doudou broker serve`; use these commands for explicit lifecycle inspection or shutdown.",
        after_help = "EXAMPLES:\n  doudou broker list\n  doudou broker status --profile work\n  doudou broker ping --profile work\n  doudou broker stop --profile work\n  doudou broker stop-all\n\nERROR CODES: BROKER_NOT_RUNNING, BROKER_START_FAILED, BROKER_START_TIMEOUT, BROKER_STOP_TIMEOUT, BROKER_AUTH_FAILED"
    )]
    Broker {
        /// Broker operation to execute.
        #[command(subcommand)]
        command: BrokerCommand,
    },

    /// Refresh, resolve, or select Douyin IM conversations (established chats and stranger message requests).
    #[command(
        long_about = "Query Douyin IM conversations from the authenticated browser protocol context. Supports filtering by conversation kind (all|group|direct) and inbox source (all|established|request for stranger message requests). Stranger requests are strictly read-only.",
        after_help = "EXAMPLES:\n  doudou conversations refresh --profile work --kind all --inbox all\n  doudou conversations refresh --profile work --inbox request --force\n  doudou conversations resolve --profile work --target \"Alpha Team\"\n  doudou conversations select --profile work --selection \"1,3-5\" --refreshed-at 2026-11-25T00:00:00.000Z\n\nERROR CODES: RECIPIENT_NOT_FOUND, CONVERSATION_AMBIGUOUS, CONVERSATION_SNAPSHOT_DRIFTED, INVALID_CONVERSATION_RANGE, CONVERSATION_NUMBER_OUT_OF_RANGE"
    )]
    Conversations {
        /// Conversation operation to execute.
        #[command(subcommand)]
        command: ConversationsCommand,
    },

    /// Collect historical and recent messages (plus encrypted images) for a single conversation.
    #[command(
        long_about = "Collect messages for one conversation at a time via Douyin IM Protobuf reads (including stranger request inbox cmd 1002). Defaults to local midnight through now when --from and --to are omitted.",
        after_help = "EXAMPLES:\n  doudou messages collect --profile work --conversation \"Alpha Team\"\n  doudou messages collect --profile work --conversation 0:1:10:20 --from 2026-11-25T00:00:00+08:00 --to 2026-11-25T23:59:59+08:00 --max-pages 50\n  doudou messages collect --profile work --conversation \"Alpha Team\" --skip-images\n\nERROR CODES: RECIPIENT_NOT_FOUND, CONVERSATION_AMBIGUOUS, MEDIA_MD5_MISMATCH, PROTOCOL_SCHEMA_DRIFTED, RATE_LIMITED"
    )]
    Messages {
        /// Message operation to execute.
        #[command(subcommand)]
        command: MessagesCommand,
    },

    /// Two-phase chat writes (send, send-batch, reply) and headed protocol traffic capture.
    #[command(
        long_about = "Create and confirm single-use write plans for chat text, image, reply, and batch messages, or capture a live Protobuf send exchange for diagnostics.\n\nTwo-phase safety: omit --confirm to create a preview plan; after user confirmation, re-run with --confirm --plan-id <id>. Never retry WRITE_RESULT_UNKNOWN automatically; verify with read-only `messages collect` first.",
        after_help = "EXAMPLES:\n  doudou protocol send --profile work --recipient \"Alpha Team\" --text \"Hello\"\n  doudou protocol send --profile work --recipient \"Alpha Team\" --image ./photo.png\n  doudou protocol send --profile work --confirm --plan-id plan-...\n  doudou protocol reply --profile work --recipient \"Alpha Team\" --message msg-1 --text \"Got it\"\n  doudou protocol send-batch --profile work --conversation-ids \"id1,id2\" --text \"Notice\"\n  doudou protocol capture --profile work --timeout 300\n\nERROR CODES: PLAN_ID_REQUIRED, PLAN_NOT_FOUND, PLAN_EXPIRED, PLAN_DRIFTED, PLAN_ALREADY_USED, STRANGER_WRITE_UNSUPPORTED, MENTION_UNSUPPORTED, WRITE_RESULT_UNKNOWN"
    )]
    Protocol {
        /// Protocol write or capture operation.
        #[command(subcommand)]
        command: ProtocolCommand,
    },

    /// Chat interaction writes (like, send, reply, send-batch/batch) and chat UI diagnostics.
    #[command(
        long_about = "Execute two-phase chat message likes, sends, replies, batch sends, or chat page UI diagnostics. Every write command generates a single-use preview plan unless --confirm --plan-id <id> is passed.",
        after_help = "EXAMPLES:\n  doudou chat like --profile work --conversation \"Alpha Team\" --message msg-123\n  doudou chat like --profile work --confirm --plan-id plan-...\n  doudou chat send --profile work --recipient \"Alpha Team\" --text \"Hi\"\n  doudou chat diagnose --profile work --conversation \"Alpha Team\"\n\nERROR CODES: PLAN_ID_REQUIRED, PLAN_ALREADY_USED, STRANGER_WRITE_UNSUPPORTED, MESSAGE_TARGET_NOT_VISIBLE, WRITE_RESULT_UNKNOWN, PAGE_ADAPTER_DRIFTED"
    )]
    Chat {
        /// Chat operation to execute.
        #[command(subcommand)]
        command: ChatCommand,
    },

    /// Resolve a Douyin ID (`unique_id`) into structured identity metadata (uid, `sec_uid`, nickname).
    #[command(
        long_about = "Resolve a Douyin ID (`unique_id`) to its canonical `sec_uid`, `uid`, and profile metadata. Fails closed with LEGACY_UNSUPPORTED if unversioned legacy identity records exist under the profile.",
        after_help = "EXAMPLES:\n  doudou identity resolve --profile work --douyin-id Rurutia.wow\n\nERROR CODES: LEGACY_UNSUPPORTED, IDENTITY_NOT_FOUND, PROTOCOL_SCHEMA_DRIFTED, RATE_LIMITED"
    )]
    Identity {
        /// Identity operation to execute.
        #[command(subcommand)]
        command: IdentityCommand,
    },

    /// Collect all videos published by a Douyin user identified by --douyin-id.
    #[command(
        long_about = "Collect a user's video list into <data_root>/message/profiles/<name>/content/users/<sec_uid>/. Uses cached identity when available or resolves --douyin-id automatically.",
        after_help = "EXAMPLES:\n  doudou videos collect --profile work --douyin-id Rurutia.wow --max-pages 100\n\nERROR CODES: IDENTITY_NOT_FOUND, PROTOCOL_SCHEMA_DRIFTED, RATE_LIMITED, VERIFICATION_REQUIRED"
    )]
    Videos {
        /// User videos operation to execute.
        #[command(subcommand)]
        command: VideosCommand,
    },

    /// Collect a video's detail/comments or perform confirmed comment create, reply, and like writes.
    #[command(
        long_about = "Collect a single video's metadata, cover image, and full comment/reply tree, or run two-phase confirmed comment, reply, and like writes verified by protocol read-back.",
        after_help = "EXAMPLES:\n  doudou video collect --profile work --sec-uid MS4w... --aweme-id 7300000000000000000\n  doudou video comment --profile work --sec-uid MS4w... --aweme-id 7300... --text \"Nice!\"\n  doudou video comment --profile work --confirm --plan-id plan-...\n  doudou video reply --profile work --sec-uid MS4w... --aweme-id 7300... --comment-id 7400... --text \"Thanks\"\n  doudou video like --profile work --sec-uid MS4w... --aweme-id 7300... --comment-id 7400...\n\nERROR CODES: VIDEO_TARGET_REQUIRED, COMMENT_TEXT_REQUIRED, COMMENT_TARGET_NOT_FOUND, COMMENT_TARGET_DRIFTED, COMMENT_TARGET_NOT_VISIBLE, PLAN_ID_REQUIRED, PLAN_ALREADY_USED, WRITE_RESULT_UNKNOWN"
    )]
    Video {
        /// Video operation to execute.
        #[command(subcommand)]
        command: VideoCommand,
    },

    /// Probe live Douyin page UI components and detect DOM/selector adapter drift.
    #[command(
        name = "page-doctor",
        visible_aliases = ["ui-probe", "diagnose"],
        long_about = "Run bounded UI component contract checks against the current Douyin chat or video page and persist a diagnostic report under diagnostics/page-doctor/. Passing --conversation opens that chat and validates composer/send controls without typing or sending.",
        after_help = "EXAMPLES:\n  doudou page-doctor --profile work\n  doudou page-doctor --profile work --conversation \"Alpha Team\"\n  doudou ui-probe --profile work\n\nERROR CODES: PAGE_ADAPTER_DRIFTED, PAGE_DOCTOR_STAGE_TIMEOUT, AUTH_LOGIN_REQUIRED, VERIFICATION_REQUIRED"
    )]
    PageDoctor(PageDoctorArgs),

    /// Self-describing CLI discovery: search commands by intent or dump machine-readable JSON schema.
    #[command(
        long_about = "Introspect the live `doudou` command tree directly from clap metadata. Use `cli search` to locate commands, flags, or error codes by intent, and `cli schema` to emit structured JSON schemas for agents and automation.",
        after_help = "EXAMPLES:\n  doudou cli search \"send\"\n  doudou cli search \"stranger\" --json\n  doudou cli search \"appdata\"\n  doudou cli schema\n  doudou cli schema doctor\n  doudou cli schema profile create"
    )]
    Cli {
        /// Introspection subcommand.
        #[command(subcommand)]
        command: CliMetaCommand,
    },
}

/// Arguments for `doudou doctor`.
#[derive(Debug, Args)]
pub struct DoctorArgs {
    /// Launch a real headless Chrome into a temp profile and verify isolated-world CDP eval without Runtime.enable.
    #[arg(long = "launch-probe", conflicts_with = "skip_launch")]
    pub launch_probe: bool,

    /// Legacy no-op alias kept for backwards compatibility with Python CLI invocations.
    #[arg(long = "skip-launch", hide = true)]
    pub skip_launch: bool,

    /// Optional explicit Chrome/Chromium/Edge binary path to validate and probe.
    #[arg(long = "chrome-binary", value_name = "PATH")]
    pub chrome_binary: Option<PathBuf>,

    /// Optional profile name to inspect alongside overall environment health.
    #[arg(long, value_name = "NAME")]
    pub profile: Option<String>,
}

/// Subcommands under `doudou profile`.
#[derive(Debug, Subcommand)]
pub enum ProfileCommand {
    /// List all current-schema (`doudou.profile.v3`) profiles with lock, chat, content, and write summaries.
    #[command(
        after_help = "EXAMPLE:\n  doudou profile list\n  doudou --home /tmp/doudou-home profile list"
    )]
    List,

    /// Create a new Chrome profile under `<doudou_home>/profiles/<name>/` and initialize project message folders.
    #[command(
        after_help = "EXAMPLE:\n  doudou profile create --profile work --display-name \"Work Account\""
    )]
    Create {
        /// Profile identifier (encoded safely for filesystem storage).
        #[arg(long, value_name = "NAME")]
        profile: String,

        /// Human-friendly display label stored in `profile.json`.
        #[arg(long = "display-name", value_name = "LABEL")]
        display_name: Option<String>,
    },

    /// Show profile metadata, storage paths (OS `AppData` + project records), and live broker status if running.
    #[command(
        visible_aliases = ["show", "info"],
        after_help = "EXAMPLE:\n  doudou profile status --profile work\n  doudou profile show --profile work"
    )]
    Status {
        /// Profile name.
        #[arg(long, value_name = "NAME")]
        profile: String,
    },

    /// Open a headed Chrome window for interactive Douyin login takeover and record verified login status.
    #[command(after_help = "EXAMPLE:\n  doudou profile login --profile work --timeout 600")]
    Login {
        /// Profile name.
        #[arg(long, value_name = "NAME")]
        profile: String,

        /// Maximum wait time in seconds for interactive login completion.
        #[arg(long, default_value_t = 600.0, value_name = "SECONDS")]
        timeout: f64,
    },

    /// Delete a profile from OS `AppData` (and optionally its project-local message records).
    #[command(
        after_help = "EXAMPLES:\n  doudou profile delete --profile work\n  doudou profile delete --profile work --with-messages"
    )]
    Delete {
        /// Profile name.
        #[arg(long, value_name = "NAME")]
        profile: String,

        /// Also delete `<data_root>/message/profiles/<name>/` collected records.
        #[arg(long = "with-messages")]
        with_messages: bool,
    },
}

/// Subcommands under `doudou session`.
#[derive(Debug, Subcommand)]
pub enum SessionCommand {
    /// Report browser mode, login state, visible verification challenge, IM init capture, and takeover status.
    #[command(after_help = "EXAMPLE:\n  doudou session status --profile work")]
    Status {
        /// Profile name.
        #[arg(long, value_name = "NAME")]
        profile: String,
    },

    /// Refresh Douyin IM initialization and conversation index without re-running interactive login.
    #[command(after_help = "EXAMPLE:\n  doudou session refresh --profile work")]
    Refresh {
        /// Profile name.
        #[arg(long, value_name = "NAME")]
        profile: String,
    },

    /// Switch to headed Chrome (or run headless QR login with `--headless`) and wait for user login.
    #[command(
        after_help = "EXAMPLES:\n  doudou session login --profile work --timeout 600\n  doudou session login --profile work --headless --timeout 180"
    )]
    Login {
        /// Profile name.
        #[arg(long, value_name = "NAME")]
        profile: String,

        /// Maximum wait time in seconds for login completion.
        #[arg(long, default_value_t = 600.0, value_name = "SECONDS")]
        timeout: f64,

        /// Keep Chrome headless, save the Douyin login QR PNG to disk, emit a `qr_code` event, and wait for scan completion.
        #[arg(long)]
        headless: bool,

        /// Print a scannable Unicode QR code to stderr (auto-enabled on interactive TTYs when `--headless` is set).
        #[arg(long = "terminal-qr")]
        terminal_qr: bool,
    },

    /// Non-blocking headless QR code and SMS authentication controls (ideal for agents, SSH, and servers).
    #[command(
        long_about = "Drive headless QR code and SMS login inside the profile's background broker without opening a local Chrome window.\n\nTypical agent/SSH flow:\n  1. `doudou session auth start --profile work` -> saves QR PNG and returns immediately while keeping the page alive.\n  2. Display the QR PNG to the user, or switch to SMS with `doudou session auth sms-send --profile work --phone <digits>`.\n  3. Poll with `doudou session auth status` / `wait`, or submit SMS code with `doudou session auth sms-verify --profile work --code <digits>`.",
        after_help = "EXAMPLES:\n  doudou session auth start --profile work\n  doudou session auth status --profile work\n  doudou session auth refresh-qr --profile work\n  doudou session auth sms-send --profile work --phone 13800138000\n  doudou session auth sms-verify --profile work --code-stdin\n  doudou session auth wait --profile work --timeout 180"
    )]
    Auth {
        /// Headless authentication operation.
        #[command(subcommand)]
        command: SessionAuthCommand,
    },

    /// Gracefully close the profile's active browser session and stop its broker process.
    #[command(
        visible_alias = "logout",
        after_help = "EXAMPLE:\n  doudou session close --profile work"
    )]
    Close {
        /// Profile name.
        #[arg(long, value_name = "NAME")]
        profile: String,
    },
}

/// Subcommands under `doudou session auth`.
#[derive(Debug, Subcommand)]
pub enum SessionAuthCommand {
    /// Open Douyin in headless Chrome, save the login QR code PNG, and return immediately while keeping the page alive.
    #[command(after_help = "EXAMPLE:\n  doudou session auth start --profile work")]
    Start {
        /// Profile name.
        #[arg(long, value_name = "NAME")]
        profile: String,

        /// Force reloading the Douyin login page even if already on douyin.com.
        #[arg(long)]
        reload: bool,

        /// Print a scannable Unicode QR code to stderr while keeping stdout pure NDJSON.
        #[arg(long = "terminal-qr")]
        terminal_qr: bool,
    },

    /// Check live headless authentication status (`authenticated`, `qr_ready`, `qr_scanned`, `qr_expired`, `sms_required`, `verification_required`).
    #[command(after_help = "EXAMPLE:\n  doudou session auth status --profile work")]
    Status {
        /// Profile name.
        #[arg(long, value_name = "NAME")]
        profile: String,
    },

    /// Refresh an expired or stale login QR code in headless Chrome and save the updated PNG.
    #[command(
        name = "refresh-qr",
        after_help = "EXAMPLE:\n  doudou session auth refresh-qr --profile work"
    )]
    RefreshQr {
        /// Profile name.
        #[arg(long, value_name = "NAME")]
        profile: String,

        /// Print a scannable Unicode QR code to stderr while keeping stdout pure NDJSON.
        #[arg(long = "terminal-qr")]
        terminal_qr: bool,
    },

    /// Request an SMS verification code on the live headless login or secondary-verify modal.
    #[command(
        name = "sms-send",
        after_help = "EXAMPLES:\n  doudou session auth sms-send --profile work --phone 13800138000\n  doudou session auth sms-send --profile work"
    )]
    SmsSend {
        /// Profile name.
        #[arg(long, value_name = "NAME")]
        profile: String,

        /// Phone number to enter before clicking '获取验证码' (omit during secondary SMS verification when already prefilled).
        #[arg(long, value_name = "PHONE")]
        phone: Option<String>,
    },

    /// Submit a user-received SMS verification code on the live headless login or secondary-verify modal.
    #[command(
        name = "sms-verify",
        after_help = "EXAMPLES:\n  doudou session auth sms-verify --profile work --code 123456\n  echo 123456 | doudou session auth sms-verify --profile work --code-stdin"
    )]
    SmsVerify {
        /// Profile name.
        #[arg(long, value_name = "NAME")]
        profile: String,

        /// Verification code digits (prefer `--code-stdin` when calling from scripts/agents to avoid argv exposure).
        #[arg(long, value_name = "CODE", conflicts_with = "code_stdin")]
        code: Option<String>,

        /// Read the SMS verification code from standard input instead of command-line arguments.
        #[arg(long = "code-stdin")]
        code_stdin: bool,
    },

    /// Wait until the active headless QR/SMS session finishes authenticating, requests SMS, or expires.
    #[command(after_help = "EXAMPLE:\n  doudou session auth wait --profile work --timeout 180")]
    Wait {
        /// Profile name.
        #[arg(long, value_name = "NAME")]
        profile: String,

        /// Maximum wait time in seconds.
        #[arg(long, default_value_t = 180.0, value_name = "SECONDS")]
        timeout: f64,

        /// Disable automatic QR code refresh when the QR expires during wait.
        #[arg(long = "no-auto-refresh")]
        no_auto_refresh: bool,
    },
}

/// Subcommands under `doudou broker`.
#[derive(Debug, Subcommand)]
pub enum BrokerCommand {
    /// List all live brokers registered under `<doudou_home>/brokers/` and prune stale entries.
    #[command(after_help = "EXAMPLE:\n  doudou broker list")]
    List,

    /// Query broker status for one profile without auto-starting it.
    #[command(after_help = "EXAMPLE:\n  doudou broker status --profile work")]
    Status {
        /// Profile name.
        #[arg(long, value_name = "NAME")]
        profile: String,
    },

    /// Ping a profile's broker (auto-starting it if not yet running).
    #[command(after_help = "EXAMPLE:\n  doudou broker ping --profile work")]
    Ping {
        /// Profile name.
        #[arg(long, value_name = "NAME")]
        profile: String,
    },

    /// Gracefully stop a profile's running broker and wait until its lock and descriptor clear.
    #[command(after_help = "EXAMPLE:\n  doudou broker stop --profile work")]
    Stop {
        /// Profile name.
        #[arg(long, value_name = "NAME")]
        profile: String,
    },

    /// Gracefully stop every live broker registered under `<doudou_home>/brokers/`.
    #[command(name = "stop-all", after_help = "EXAMPLE:\n  doudou broker stop-all")]
    StopAll,

    /// Start a profile broker (optionally with an in-memory fake browser) and return its status.
    #[command(
        name = "test-start",
        after_help = "EXAMPLE:\n  doudou broker test-start --profile work --fake-browser"
    )]
    TestStart {
        /// Profile name.
        #[arg(long, value_name = "NAME")]
        profile: String,

        /// Run with an in-memory fake authenticated browser session instead of launching Chrome.
        #[arg(long = "fake-browser")]
        fake_browser: bool,
    },

    /// Run the per-profile loopback TCP broker server in the foreground (invoked by auto-launcher).
    #[command(
        long_about = "Serve the authenticated loopback TCP JSON-line broker for a single profile until idle timeout, shutdown RPC, or Ctrl-C. Invoked automatically by `BrokerLauncher` when runtime commands need a broker.",
        after_help = "EXAMPLE:\n  doudou broker serve --profile work --idle-timeout 1200"
    )]
    Serve {
        /// Profile name.
        #[arg(long, value_name = "NAME")]
        profile: String,

        /// Idle timeout in seconds before the broker exits automatically.
        #[arg(
            long = "idle-timeout",
            default_value_t = 1200.0,
            value_name = "SECONDS"
        )]
        idle_timeout: f64,

        /// Use an in-memory fake browser session instead of launching Chrome.
        #[arg(long = "fake-browser")]
        fake_browser: bool,

        /// Optional explicit Chrome executable override.
        #[arg(long = "chrome-binary", value_name = "PATH")]
        chrome_binary: Option<PathBuf>,
    },
}

/// Filter for conversation type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum ConversationKindFilter {
    /// Include both direct messages and group chats.
    All,
    /// Only group chats (`conversation_type = 2`).
    Group,
    /// Only direct messages (`conversation_type = 1`).
    Direct,
}

impl ConversationKindFilter {
    /// Return the wire protocol label expected by the broker.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::All => "all",
            Self::Group => "group",
            Self::Direct => "direct",
        }
    }
}

/// Filter for conversation inbox source (established vs. stranger message requests).
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum InboxFilter {
    /// Merge established conversations and read-only stranger message requests.
    All,
    /// Only established conversations (writable).
    Established,
    /// Only stranger message requests (strictly read-only).
    Request,
}

impl InboxFilter {
    /// Return the wire protocol label expected by the broker.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::All => "all",
            Self::Established => "established",
            Self::Request => "request",
        }
    }
}

/// Subcommands under `doudou conversations`.
#[derive(Debug, Subcommand)]
pub enum ConversationsCommand {
    /// Refresh and list conversations (established chats and stranger message requests).
    #[command(
        after_help = "EXAMPLES:\n  doudou conversations refresh --profile work --kind all --inbox all\n  doudou conversations refresh --profile work --kind group --inbox established --force\n  doudou conversations refresh --profile work --inbox request"
    )]
    Refresh {
        /// Profile name.
        #[arg(long, value_name = "NAME")]
        profile: String,

        /// Filter by conversation kind (`all`, `group`, `direct`).
        #[arg(long, value_enum, default_value_t = ConversationKindFilter::All)]
        kind: ConversationKindFilter,

        /// Filter by inbox source (`all`, `established`, `request` for stranger message requests).
        #[arg(long, value_enum, default_value_t = InboxFilter::All)]
        inbox: InboxFilter,

        /// Force a live protocol refresh even if the cached session index is fresh.
        #[arg(long)]
        force: bool,
    },

    /// Resolve a conversation ID or display name unambiguously against the conversation index.
    #[command(
        after_help = "EXAMPLE:\n  doudou conversations resolve --profile work --target \"Alpha Team\""
    )]
    Resolve {
        /// Profile name.
        #[arg(long, value_name = "NAME")]
        profile: String,

        /// Conversation ID or display name substring.
        #[arg(long, value_name = "ID_OR_NAME")]
        target: String,
    },

    /// Select numbered conversations from a specific `refreshed_at` snapshot (e.g. `1,3-5` or `all`).
    #[command(
        after_help = "EXAMPLE:\n  doudou conversations select --profile work --selection \"1,3-5\" --refreshed-at 2026-11-25T00:00:00.000Z"
    )]
    Select {
        /// Profile name.
        #[arg(long, value_name = "NAME")]
        profile: String,

        /// Selection expression (`all`, `全部`, or 1-based comma/range list like `1,3-5`).
        #[arg(long, value_name = "EXPR")]
        selection: String,

        /// Snapshot timestamp from `conversations refresh` to prevent index drift.
        #[arg(long = "refreshed-at", value_name = "RFC3339")]
        refreshed_at: String,
    },
}

/// Subcommands under `doudou messages`.
#[derive(Debug, Subcommand)]
pub enum MessagesCommand {
    /// Collect messages (and optionally decrypt/download images) for a single conversation.
    #[command(
        after_help = "EXAMPLES:\n  doudou messages collect --profile work --conversation \"Alpha Team\"\n  doudou messages collect --profile work --conversation 0:1:10:20 --from 2026-11-25T00:00:00+08:00 --to 2026-11-25T18:00:00+08:00 --max-pages 50 --skip-images"
    )]
    Collect {
        /// Profile name.
        #[arg(long, value_name = "NAME")]
        profile: String,

        /// Single conversation ID or unambiguous display name (stranger requests supported read-only).
        #[arg(long, value_name = "ID_OR_NAME")]
        conversation: String,

        /// Lower bound timestamp (RFC3339). Defaults with --to to local midnight through now.
        #[arg(long = "from", value_name = "RFC3339")]
        from_time: Option<String>,

        /// Upper bound timestamp (RFC3339). Defaults with --from to local midnight through now.
        #[arg(long = "to", value_name = "RFC3339")]
        to_time: Option<String>,

        /// Maximum pagination requests to fetch.
        #[arg(long = "max-pages", default_value_t = 100, value_name = "N")]
        max_pages: usize,

        /// Skip downloading and AES-GCM decrypting message images.
        #[arg(long = "skip-images")]
        skip_images: bool,
    },
}

/// Shared arguments for `protocol send` and `chat send`.
#[derive(Debug, Args)]
pub struct SendArgs {
    /// Profile name.
    #[arg(long, value_name = "NAME")]
    pub profile: String,

    /// Conversation display name or ID (alternative to --conversation-id).
    #[arg(long, value_name = "CONVERSATION")]
    pub recipient: Option<String>,

    /// Explicit conversation ID (takes precedence over --recipient).
    #[arg(long = "conversation-id", value_name = "ID")]
    pub conversation_id: Option<String>,

    /// Message text body to preview or send.
    #[arg(long, value_name = "TEXT")]
    pub text: Option<String>,

    /// Local image file path to attach and send (two-phase confirmed).
    #[arg(long, value_name = "PATH")]
    pub image: Option<PathBuf>,

    /// Deprecated group mention flag; always fails closed before broker start with `MENTION_UNSUPPORTED`.
    #[arg(long = "at", action = clap::ArgAction::Append, hide = true)]
    pub at: Vec<String>,

    /// Optional message ID or text snippet to reply to.
    #[arg(long, value_name = "MESSAGE")]
    pub reply: Option<String>,

    /// Execute a previously created write plan once (requires --plan-id).
    #[arg(long)]
    pub confirm: bool,

    /// Single-use write plan ID returned by the preview step.
    #[arg(long = "plan-id", value_name = "ID")]
    pub plan_id: Option<String>,
}

/// Shared arguments for `protocol send-batch` and `chat send-batch`.
#[derive(Debug, Args)]
pub struct SendBatchArgs {
    /// Profile name.
    #[arg(long, value_name = "NAME")]
    pub profile: String,

    /// Comma-separated conversation names or IDs.
    #[arg(long, value_name = "CSV")]
    pub recipients: Option<String>,

    /// Comma-separated explicit conversation IDs (takes precedence over --recipients).
    #[arg(long = "conversation-ids", value_name = "CSV")]
    pub conversation_ids: Option<String>,

    /// Message text body sent to each target (spaced by >= 5s; stops on first failure).
    #[arg(long, value_name = "TEXT")]
    pub text: Option<String>,

    /// Execute a previously created batch write plan once (requires --plan-id).
    #[arg(long)]
    pub confirm: bool,

    /// Single-use batch write plan ID returned by the preview step.
    #[arg(long = "plan-id", value_name = "ID")]
    pub plan_id: Option<String>,
}

/// Shared arguments for `protocol reply` and `chat reply`.
#[derive(Debug, Args)]
pub struct ReplyArgs {
    /// Profile name.
    #[arg(long, value_name = "NAME")]
    pub profile: String,

    /// Conversation display name or ID.
    #[arg(long, value_name = "CONVERSATION")]
    pub recipient: Option<String>,

    /// Explicit conversation ID (takes precedence over --recipient).
    #[arg(long = "conversation-id", value_name = "ID")]
    pub conversation_id: Option<String>,

    /// Target message ID or unambiguous text snippet from collected records.
    #[arg(long, value_name = "MESSAGE")]
    pub message: Option<String>,

    /// Reply text body.
    #[arg(long, value_name = "TEXT")]
    pub text: Option<String>,

    /// Execute a previously created reply write plan once (requires --plan-id).
    #[arg(long)]
    pub confirm: bool,

    /// Single-use write plan ID returned by the preview step.
    #[arg(long = "plan-id", value_name = "ID")]
    pub plan_id: Option<String>,
}

/// Subcommands under `doudou protocol`.
#[derive(Debug, Subcommand)]
pub enum ProtocolCommand {
    /// Preview or confirm sending a chat text, reply, or image message.
    #[command(
        after_help = "EXAMPLES:\n  doudou protocol send --profile work --recipient \"Alpha Team\" --text \"Hello\"\n  doudou protocol send --profile work --recipient \"Alpha Team\" --image ./photo.png\n  doudou protocol send --profile work --confirm --plan-id plan-..."
    )]
    Send(SendArgs),

    /// Preview or confirm sending the same text message to multiple conversations sequentially.
    #[command(
        name = "send-batch",
        after_help = "EXAMPLES:\n  doudou protocol send-batch --profile work --conversation-ids \"id1,id2\" --text \"Notice\"\n  doudou protocol send-batch --profile work --confirm --plan-id plan-..."
    )]
    SendBatch(SendBatchArgs),

    /// Preview or confirm replying to a specific collected chat message.
    #[command(
        after_help = "EXAMPLES:\n  doudou protocol reply --profile work --recipient \"Alpha Team\" --message msg-1 --text \"Got it\"\n  doudou protocol reply --profile work --confirm --plan-id plan-..."
    )]
    Reply(ReplyArgs),

    /// Open headed Chrome and capture one real `/v1/message/send` Protobuf request/response pair.
    #[command(after_help = "EXAMPLE:\n  doudou protocol capture --profile work --timeout 300")]
    Capture {
        /// Profile name.
        #[arg(long, value_name = "NAME")]
        profile: String,

        /// Maximum wait time in seconds to observe one complete send exchange.
        #[arg(long, default_value_t = 600.0, value_name = "SECONDS")]
        timeout: f64,
    },
}

/// Subcommands under `doudou chat`.
#[derive(Debug, Subcommand)]
pub enum ChatCommand {
    /// Preview or confirm liking a collected chat message (verified by protocol read-back).
    #[command(
        after_help = "EXAMPLES:\n  doudou chat like --profile work --conversation \"Alpha Team\" --message msg-1\n  doudou chat like --profile work --confirm --plan-id plan-..."
    )]
    Like {
        /// Profile name.
        #[arg(long, value_name = "NAME")]
        profile: String,

        /// Conversation ID or unambiguous display name (must be established, not a stranger request).
        #[arg(long, value_name = "ID_OR_NAME")]
        conversation: Option<String>,

        /// Message ID or unambiguous text snippet from collected records.
        #[arg(long, value_name = "MESSAGE")]
        message: Option<String>,

        /// Execute a previously created like write plan once (requires --plan-id).
        #[arg(long)]
        confirm: bool,

        /// Single-use write plan ID returned by the preview step.
        #[arg(long = "plan-id", value_name = "ID")]
        plan_id: Option<String>,
    },

    /// Preview or confirm sending a chat text, reply, or image message (alias of `protocol send`).
    #[command(
        after_help = "EXAMPLES:\n  doudou chat send --profile work --recipient \"Alpha Team\" --text \"Hello\"\n  doudou chat send --profile work --confirm --plan-id plan-..."
    )]
    Send(SendArgs),

    /// Preview or confirm replying to a collected chat message (alias of `protocol reply`).
    #[command(
        after_help = "EXAMPLES:\n  doudou chat reply --profile work --recipient \"Alpha Team\" --message msg-1 --text \"Got it\"\n  doudou chat reply --profile work --confirm --plan-id plan-..."
    )]
    Reply(ReplyArgs),

    /// Preview or confirm sending a text message to multiple conversations (alias of `protocol send-batch`).
    #[command(
        name = "send-batch",
        visible_alias = "batch",
        after_help = "EXAMPLES:\n  doudou chat batch --profile work --conversation-ids \"id1,id2\" --text \"Notice\"\n  doudou chat batch --profile work --confirm --plan-id plan-..."
    )]
    SendBatch(SendBatchArgs),

    /// Run chat UI component drift diagnostics (alias of `page-doctor`).
    #[command(
        after_help = "EXAMPLE:\n  doudou chat diagnose --profile work --conversation \"Alpha Team\""
    )]
    Diagnose(PageDoctorArgs),
}

/// Subcommands under `doudou identity`.
#[derive(Debug, Subcommand)]
pub enum IdentityCommand {
    /// Resolve a Douyin ID (`unique_id`) to `sec_uid`, `uid`, and profile metadata.
    #[command(
        after_help = "EXAMPLE:\n  doudou identity resolve --profile work --douyin-id Rurutia.wow"
    )]
    Resolve {
        /// Profile name.
        #[arg(long, value_name = "NAME")]
        profile: String,

        /// Douyin ID (`unique_id`, not display nickname or `sec_uid`).
        #[arg(long = "douyin-id", value_name = "DOUYIN_ID")]
        douyin_id: String,
    },
}

/// Subcommands under `doudou videos`.
#[derive(Debug, Subcommand)]
pub enum VideosCommand {
    /// Collect all videos published by a user identified by `--douyin-id`.
    #[command(
        after_help = "EXAMPLE:\n  doudou videos collect --profile work --douyin-id Rurutia.wow --max-pages 100"
    )]
    Collect {
        /// Profile name.
        #[arg(long, value_name = "NAME")]
        profile: String,

        /// Douyin ID (`unique_id`).
        #[arg(long = "douyin-id", value_name = "DOUYIN_ID")]
        douyin_id: String,

        /// Maximum video list pages to fetch.
        #[arg(long = "max-pages", default_value_t = 1000, value_name = "N")]
        max_pages: usize,
    },
}

/// Subcommands under `doudou video`.
#[derive(Debug, Subcommand)]
pub enum VideoCommand {
    /// Collect a video's detail, cover image, and complete comment/reply tree.
    #[command(
        after_help = "EXAMPLE:\n  doudou video collect --profile work --sec-uid MS4w... --aweme-id 7300000000000000000"
    )]
    Collect {
        /// Profile name.
        #[arg(long, value_name = "NAME")]
        profile: String,

        /// Author `sec_uid` from `identity resolve`.
        #[arg(long = "sec-uid", value_name = "SEC_UID")]
        sec_uid: String,

        /// Target video `aweme_id`.
        #[arg(long = "aweme-id", value_name = "AWEME_ID")]
        aweme_id: String,

        /// Maximum comment/reply pages to fetch.
        #[arg(long = "max-pages", default_value_t = 1000, value_name = "N")]
        max_pages: usize,
    },

    /// Preview or confirm posting a top-level comment on a video.
    #[command(
        after_help = "EXAMPLES:\n  doudou video comment --profile work --sec-uid MS4w... --aweme-id 7300... --text \"Great video!\"\n  doudou video comment --profile work --confirm --plan-id plan-..."
    )]
    Comment {
        /// Profile name.
        #[arg(long, value_name = "NAME")]
        profile: String,

        /// Author `sec_uid`.
        #[arg(long = "sec-uid", value_name = "SEC_UID")]
        sec_uid: Option<String>,

        /// Target video `aweme_id`.
        #[arg(long = "aweme-id", value_name = "AWEME_ID")]
        aweme_id: Option<String>,

        /// Comment text body.
        #[arg(long, value_name = "TEXT")]
        text: Option<String>,

        /// Execute a previously created comment write plan once (requires --plan-id).
        #[arg(long)]
        confirm: bool,

        /// Single-use write plan ID returned by the preview step.
        #[arg(long = "plan-id", value_name = "ID")]
        plan_id: Option<String>,
    },

    /// Preview or confirm replying to an existing collected video comment.
    #[command(
        after_help = "EXAMPLES:\n  doudou video reply --profile work --sec-uid MS4w... --aweme-id 7300... --comment-id 7400... --text \"Thanks!\"\n  doudou video reply --profile work --confirm --plan-id plan-..."
    )]
    Reply {
        /// Profile name.
        #[arg(long, value_name = "NAME")]
        profile: String,

        /// Author `sec_uid`.
        #[arg(long = "sec-uid", value_name = "SEC_UID")]
        sec_uid: Option<String>,

        /// Target video `aweme_id`.
        #[arg(long = "aweme-id", value_name = "AWEME_ID")]
        aweme_id: Option<String>,

        /// Reply text body.
        #[arg(long, value_name = "TEXT")]
        text: Option<String>,

        /// Target comment `cid` from collected video comments.
        #[arg(long = "comment-id", value_name = "CID")]
        comment_id: Option<String>,

        /// Optional expected comment text used to detect drift before confirmation.
        #[arg(long = "comment-text", value_name = "TEXT")]
        comment_text: Option<String>,

        /// Execute a previously created reply write plan once (requires --plan-id).
        #[arg(long)]
        confirm: bool,

        /// Single-use write plan ID returned by the preview step.
        #[arg(long = "plan-id", value_name = "ID")]
        plan_id: Option<String>,
    },

    /// Preview or confirm liking a collected video comment (verified by protocol read-back).
    #[command(
        after_help = "EXAMPLES:\n  doudou video like --profile work --sec-uid MS4w... --aweme-id 7300... --comment-id 7400...\n  doudou video like --profile work --confirm --plan-id plan-..."
    )]
    Like {
        /// Profile name.
        #[arg(long, value_name = "NAME")]
        profile: String,

        /// Author `sec_uid`.
        #[arg(long = "sec-uid", value_name = "SEC_UID")]
        sec_uid: Option<String>,

        /// Target video `aweme_id`.
        #[arg(long = "aweme-id", value_name = "AWEME_ID")]
        aweme_id: Option<String>,

        /// Target comment `cid` from collected video comments.
        #[arg(long = "comment-id", value_name = "CID")]
        comment_id: Option<String>,

        /// Optional expected comment text used to detect drift before confirmation.
        #[arg(long = "comment-text", value_name = "TEXT")]
        comment_text: Option<String>,

        /// Optional expected digg count from collected comment metadata.
        #[arg(long = "digg-count", default_value_t = 0, value_name = "N")]
        digg_count: u64,

        /// Mark whether the comment was already liked in the collected snapshot.
        #[arg(long = "already-liked")]
        already_liked: bool,

        /// Execute a previously created like write plan once (requires --plan-id).
        #[arg(long)]
        confirm: bool,

        /// Single-use write plan ID returned by the preview step.
        #[arg(long = "plan-id", value_name = "ID")]
        plan_id: Option<String>,
    },
}

/// Arguments for `doudou page-doctor` / `ui-probe` / `diagnose`.
#[derive(Debug, Args)]
pub struct PageDoctorArgs {
    /// Profile name.
    #[arg(long, value_name = "NAME")]
    pub profile: String,

    /// Optional baseline report JSON file path for comparison.
    #[arg(long, value_name = "PATH")]
    pub baseline: Option<PathBuf>,

    /// Optional conversation name to open and probe composer/send controls (read-only, never sends).
    #[arg(long, value_name = "CONVERSATION")]
    pub conversation: Option<String>,
}

/// Self-describing discovery subcommands under `doudou cli`.
#[derive(Debug, Subcommand)]
pub enum CliMetaCommand {
    /// Search commands, aliases, flags, examples, and error codes by intent.
    #[command(
        after_help = "EXAMPLES:\n  doudou cli search \"send\"\n  doudou cli search \"like\" --json\n  doudou cli search \"stranger\"\n  doudou cli search \"appdata\""
    )]
    Search {
        /// Free-text search query (one or more words matching command, flag, intent, or error code).
        #[arg(required = true, num_args = 1.., value_name = "QUERY")]
        query: Vec<String>,

        /// Emit structured machine-readable JSON instead of human-readable ranked output.
        #[arg(long)]
        json: bool,

        /// Maximum number of ranked command matches to return.
        #[arg(long, default_value_t = 12, value_name = "N")]
        limit: usize,
    },

    /// Emit machine-readable JSON schema introspected directly from the live `clap::Command` tree.
    #[command(
        after_help = "EXAMPLES:\n  doudou cli schema\n  doudou cli schema doctor\n  doudou cli schema profile create\n  doudou cli schema protocol send"
    )]
    Schema {
        /// Optional command path to scope the schema (e.g. `doctor`, `profile create`, `protocol send`).
        #[arg(value_name = "COMMAND")]
        command: Vec<String>,
    },
}
