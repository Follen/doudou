//! Self-describing `doudou` CLI crate combining 100% Python CLI parity with
//! `cf`-style live `clap::Command` introspection (`cli search` and `cli schema`).

pub mod args;
pub mod dispatch;
pub mod introspect;

use std::io::Write as _;

use clap::Parser as _;
use doudou_core::emit;
use serde_json::{Value, json};

pub use args::{
    BrokerCommand, ChatCommand, Cli, CliMetaCommand, Commands, ConversationKindFilter,
    ConversationsCommand, DoctorArgs, IdentityCommand, InboxFilter, MessagesCommand,
    PageDoctorArgs, ProfileCommand, ProtocolCommand, ReplyArgs, SendArgs, SendBatchArgs,
    SessionCommand, VideoCommand, VideosCommand,
};
pub use dispatch::{command_name, dispatch, dispatch_with_roots, resolve_roots};
pub use introspect::{
    ArgSchema, CLI_SCHEMA_VERSION, CommandSchema, ErrorCodeDoc, SearchMatch, SearchResponse,
    build_schema, error_code_catalog, format_search_text, search_cli,
};

/// Execute the `doudou` CLI from process arguments and return the POSIX/Windows exit code.
pub async fn run() -> i32 {
    disable_std_handle_inheritance();
    let cli = Cli::parse();
    run_parsed(&cli).await
}

/// Prevent detached child processes (such as `doudou broker serve` or Chrome) from inheriting
/// the caller's stdout/stderr capture pipes on Windows, which would otherwise block pipe EOF.
#[cfg(windows)]
fn disable_std_handle_inheritance() {
    const STD_INPUT_HANDLE: u32 = 0xFFFF_FFF6;
    const STD_OUTPUT_HANDLE: u32 = 0xFFFF_FFF5;
    const STD_ERROR_HANDLE: u32 = 0xFFFF_FFF4;
    const HANDLE_FLAG_INHERIT: u32 = 0x0000_0001;

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetStdHandle(n_std_handle: u32) -> isize;
        fn SetHandleInformation(h_object: isize, dw_mask: u32, dw_flags: u32) -> i32;
    }

    for std_handle in [STD_INPUT_HANDLE, STD_OUTPUT_HANDLE, STD_ERROR_HANDLE] {
        // SAFETY: `GetStdHandle` and `SetHandleInformation` are standard Win32 kernel32 APIs;
        // null (0) and `INVALID_HANDLE_VALUE` (-1) are excluded before modifying handle flags.
        unsafe {
            let handle = GetStdHandle(std_handle);
            if handle != 0 && handle != -1 {
                let _ = SetHandleInformation(handle, HANDLE_FLAG_INHERIT, 0);
            }
        }
    }
}

#[cfg(not(windows))]
fn disable_std_handle_inheritance() {}

/// Execute an already-parsed [`Cli`] instance, emitting NDJSON for runtime commands or
/// direct output for `cli search` / `cli schema`, and return the exit code.
pub async fn run_parsed(cli: &Cli) -> i32 {
    if let Commands::Cli { command } = &cli.command {
        return run_meta(command);
    }

    let (home, data_root) = match resolve_roots(cli) {
        Ok(roots) => roots,
        Err(err) => {
            emit(&err.as_event());
            return err.exit_code;
        }
    };

    emit(&json!({
        "event": "started",
        "command": command_name(&cli.command),
        "home": home.display().to_string(),
        "data_root": data_root.display().to_string(),
        "version": env!("CARGO_PKG_VERSION"),
    }));

    match dispatch_with_roots(&cli.command, &home, &data_root).await {
        Ok(result) => {
            let failed = result.get("status").and_then(Value::as_str) == Some("failed");
            emit(&json!({
                "event": "result",
                "result": result,
            }));
            i32::from(failed)
        }
        Err(err) => {
            emit(&err.as_event());
            err.exit_code
        }
    }
}

fn run_meta(command: &CliMetaCommand) -> i32 {
    match command {
        CliMetaCommand::Search { query, json, limit } => {
            let joined = query.join(" ");
            let response = search_cli(&joined, *limit);
            let stdout = std::io::stdout();
            let mut handle = stdout.lock();
            if *json {
                match serde_json::to_string_pretty(&response) {
                    Ok(rendered) => {
                        let _ = writeln!(handle, "{rendered}");
                        0
                    }
                    Err(err) => {
                        let error = doudou_core::DoudouError::new(
                            "JSON_SERIALIZE_ERROR",
                            format!("Failed serializing search output: {err}"),
                        );
                        emit(&error.as_event());
                        error.exit_code
                    }
                }
            } else {
                let _ = write!(handle, "{}", format_search_text(&response));
                0
            }
        }
        CliMetaCommand::Schema { command } => match build_schema(command) {
            Ok(schema) => {
                let stdout = std::io::stdout();
                let mut handle = stdout.lock();
                match serde_json::to_string_pretty(&schema) {
                    Ok(rendered) => {
                        let _ = writeln!(handle, "{rendered}");
                        0
                    }
                    Err(err) => {
                        let error = doudou_core::DoudouError::new(
                            "JSON_SERIALIZE_ERROR",
                            format!("Failed serializing CLI schema: {err}"),
                        );
                        emit(&error.as_event());
                        error.exit_code
                    }
                }
            }
            Err(err) => {
                emit(&err.as_event());
                err.exit_code
            }
        },
    }
}
