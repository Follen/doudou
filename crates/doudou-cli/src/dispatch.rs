//! Async command dispatcher wiring `doudou-cli` arguments to `doudou-core`,
//! `doudou-storage`, and `doudou-runtime`.

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use doudou_core::{
    DoudouError, Result, absolutize_path, assert_supported_identity_records, doudou_home,
    initialize_roots, parse_selection, resolve_conversation, resolve_data_root, utc_now,
};
use doudou_runtime::{
    BrokerClient, BrokerLauncher, BrokerServer, BrokerServerConfig, CommandSemantics,
    DefaultBrokerHandler, DoctorOptions, broker_status, doctor, list_brokers, lock_is_live,
    stop_all_brokers, stop_broker,
};
use doudou_storage::{
    DeleteScope, create_profile, delete_profile, list_profiles, load_profile, update_profile,
};
use serde_json::{Map, Value, json};

use crate::args::{
    BrokerCommand, ChatCommand, Cli, Commands, ConversationsCommand, DoctorArgs, IdentityCommand,
    MessagesCommand, PageDoctorArgs, ProfileCommand, ProtocolCommand, ReplyArgs, SendArgs,
    SendBatchArgs, SessionCommand, VideoCommand, VideosCommand,
};

fn build_broker_client(
    home: &Path,
    data_root: &Path,
    profile: &str,
    fake: bool,
) -> Result<BrokerClient> {
    let launcher = BrokerLauncher::current_exe(vec!["broker".to_owned(), "serve".to_owned()])?;
    Ok(BrokerClient::new(home, data_root, profile)?
        .with_fake_browser(fake)
        .with_launcher(launcher))
}

fn require_target(
    conversation_id: Option<&str>,
    recipient: Option<&str>,
    conversation: Option<&str>,
) -> Result<String> {
    conversation_id
        .or(recipient)
        .or(conversation)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .ok_or_else(|| {
            DoudouError::new(
                "MISSING_ARGUMENT",
                "A conversation ID or existing conversation name is required.",
            )
        })
}

fn parse_mentions(values: &[String]) -> Vec<String> {
    values
        .iter()
        .flat_map(|entry| entry.split(','))
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}

fn require_plan_id(plan_id: Option<&str>) -> Result<&str> {
    plan_id
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| DoudouError::new("PLAN_ID_REQUIRED", "--confirm requires --plan-id."))
}

fn broker_result(response: &Value) -> Value {
    response.get("result").cloned().unwrap_or(Value::Null)
}

/// Return the top-level command name used in the `started` NDJSON event.
pub const fn command_name(command: &Commands) -> &'static str {
    match command {
        Commands::Doctor(_) => "doctor",
        Commands::Profile { .. } => "profile",
        Commands::Session { .. } => "session",
        Commands::Broker { .. } => "broker",
        Commands::Conversations { .. } => "conversations",
        Commands::Messages { .. } => "messages",
        Commands::Protocol { .. } => "protocol",
        Commands::Chat { .. } => "chat",
        Commands::Identity { .. } => "identity",
        Commands::Videos { .. } => "videos",
        Commands::Video { .. } => "video",
        Commands::PageDoctor(_) => "page-doctor",
        Commands::Cli { .. } => "cli",
    }
}

/// Resolve `(home, data_root)` from global CLI options, environment variables, and OS defaults.
pub fn resolve_roots(cli: &Cli) -> Result<(std::path::PathBuf, std::path::PathBuf)> {
    let home = doudou_home(cli.home.as_deref());
    let data_root = resolve_data_root(cli.data_root.as_deref())?;
    Ok((home, data_root))
}

/// Dispatch any parsed runtime command and return its structured JSON result payload.
pub async fn dispatch(cli: &Cli) -> Result<Value> {
    let (home, data_root) = resolve_roots(cli)?;
    dispatch_with_roots(&cli.command, &home, &data_root).await
}

/// Dispatch a parsed command using explicit resolved `(home, data_root)` paths.
pub async fn dispatch_with_roots(
    command: &Commands,
    home: &Path,
    data_root: &Path,
) -> Result<Value> {
    match command {
        Commands::Doctor(args) => handle_doctor(home, data_root, args).await,
        Commands::Identity { command } => handle_identity(home, data_root, command).await,
        Commands::Profile { command } => {
            initialize_roots(home, data_root)?;
            handle_profile(home, data_root, command).await
        }
        Commands::Session { command } => {
            initialize_roots(home, data_root)?;
            handle_session(home, data_root, command).await
        }
        Commands::Broker { command } => {
            initialize_roots(home, data_root)?;
            handle_broker(home, data_root, command).await
        }
        Commands::Conversations { command } => {
            initialize_roots(home, data_root)?;
            handle_conversations(home, data_root, command).await
        }
        Commands::Messages { command } => {
            initialize_roots(home, data_root)?;
            handle_messages(home, data_root, command).await
        }
        Commands::Protocol { command } => {
            initialize_roots(home, data_root)?;
            handle_protocol(home, data_root, command).await
        }
        Commands::Chat { command } => {
            initialize_roots(home, data_root)?;
            handle_chat(home, data_root, command).await
        }
        Commands::Videos { command } => {
            initialize_roots(home, data_root)?;
            handle_videos(home, data_root, command).await
        }
        Commands::Video { command } => {
            initialize_roots(home, data_root)?;
            handle_video(home, data_root, command).await
        }
        Commands::PageDoctor(args) => {
            initialize_roots(home, data_root)?;
            handle_page_doctor(home, data_root, args).await
        }
        Commands::Cli { .. } => Err(DoudouError::new(
            "INTERNAL_ERROR",
            "Meta `cli` commands must be handled before runtime dispatch.",
        )),
    }
}

async fn handle_doctor(home: &Path, data_root: &Path, args: &DoctorArgs) -> Result<Value> {
    initialize_roots(home, data_root)?;
    let mut options = DoctorOptions::new().with_launch_probe(args.launch_probe);
    if let Some(binary) = &args.chrome_binary {
        options = options.with_chrome_binary(binary.clone());
    }
    let mut report = doctor(home, data_root, &options).await?;
    if let Some(profile_name) = &args.profile {
        let (metadata, paths) = load_profile(home, data_root, profile_name)?;
        let broker = broker_status(home, data_root, profile_name).await?;
        if let Some(obj) = report.as_object_mut() {
            obj.insert(
                "target_profile".to_owned(),
                json!({
                    "profile": metadata,
                    "broker": broker,
                    "paths": {
                        "profile": paths.profile.display().to_string(),
                        "browser_data": paths.browser_data.display().to_string(),
                        "message": paths.message.display().to_string(),
                    }
                }),
            );
        }
    }
    Ok(report)
}

async fn run_interactive_login(
    home: &Path,
    data_root: &Path,
    profile_name: &str,
    timeout: f64,
) -> Result<Value> {
    let (_metadata, _paths) = load_profile(home, data_root, profile_name)?;
    let client = build_broker_client(home, data_root, profile_name, false)?;
    client
        .request_async(
            "browser.visible",
            json!({ "visible": true }),
            CommandSemantics::Control,
            None,
            true,
        )
        .await?;
    client
        .request_async(
            "page.open",
            json!({ "url": "https://www.douyin.com/" }),
            CommandSemantics::Control,
            None,
            true,
        )
        .await?;
    let resp = client
        .request_async(
            "login.wait",
            json!({ "timeout_seconds": timeout }),
            CommandSemantics::Control,
            None,
            true,
        )
        .await?;
    let result = broker_result(&resp);
    let verified_at = result
        .get("initialized_at")
        .and_then(Value::as_str)
        .map_or_else(utc_now, ToOwned::to_owned);

    let mut patch = Map::new();
    patch.insert("login_status".to_owned(), json!("valid"));
    patch.insert("last_login_verified_at".to_owned(), json!(verified_at));
    patch.insert("last_error_code".to_owned(), Value::Null);
    let profile_record = update_profile(home, data_root, profile_name, &patch)?;

    Ok(json!({
        "status": "logged_in",
        "profile": profile_record,
        "session": result,
    }))
}

async fn handle_profile(home: &Path, data_root: &Path, command: &ProfileCommand) -> Result<Value> {
    match command {
        ProfileCommand::List => {
            let profiles = list_profiles(home, data_root, lock_is_live)?;
            Ok(json!({
                "status": "ok",
                "count": profiles.len(),
                "profiles": profiles,
            }))
        }
        ProfileCommand::Create {
            profile,
            display_name,
        } => {
            let (metadata, paths) =
                create_profile(home, data_root, profile, display_name.as_deref())?;
            Ok(json!({
                "status": "created",
                "profile": metadata,
                "paths": {
                    "profile": paths.profile.display().to_string(),
                    "browser_data": paths.browser_data.display().to_string(),
                    "browser_config": paths.browser_config.display().to_string(),
                    "message": paths.message.display().to_string(),
                }
            }))
        }
        ProfileCommand::Status { profile } => {
            let (metadata, paths) = load_profile(home, data_root, profile)?;
            let client = build_broker_client(home, data_root, profile, false)?;
            let broker = if client.descriptor().is_some() {
                let resp = client
                    .request_async("status", json!({}), CommandSemantics::Read, None, false)
                    .await?;
                broker_result(&resp)
            } else {
                Value::Null
            };
            Ok(json!({
                "status": "ok",
                "profile": metadata,
                "broker": broker,
                "paths": {
                    "profile": paths.profile.display().to_string(),
                    "browser_data": paths.browser_data.display().to_string(),
                    "message": paths.message.display().to_string(),
                }
            }))
        }
        ProfileCommand::Login { profile, timeout } => {
            run_interactive_login(home, data_root, profile, *timeout).await
        }
        ProfileCommand::Delete {
            profile,
            with_messages,
        } => {
            let (_metadata, paths) = load_profile(home, data_root, profile)?;
            if lock_is_live(&paths.lock) {
                return Err(DoudouError::new(
                    "PROFILE_LOCKED",
                    format!(
                        "Profile `{}` is currently in use by a live broker. Stop it first with `doudou broker stop --profile {}`.",
                        paths.profile_id, profile
                    ),
                )
                .with_details(json!({ "profile_id": paths.profile_id })));
            }
            let scope = if *with_messages {
                DeleteScope::ProfileAndMessages
            } else {
                DeleteScope::ProfileOnly
            };
            let deleted = delete_profile(home, data_root, profile, scope)?;
            let mut out = deleted.as_object().cloned().unwrap_or_default();
            out.insert("status".to_owned(), json!("deleted"));
            Ok(Value::Object(out))
        }
    }
}

async fn handle_session(home: &Path, data_root: &Path, command: &SessionCommand) -> Result<Value> {
    match command {
        SessionCommand::Status { profile } => {
            let client = build_broker_client(home, data_root, profile, false)?;
            let resp = client
                .request_async(
                    "session.status",
                    json!({}),
                    CommandSemantics::Read,
                    None,
                    true,
                )
                .await?;
            Ok(broker_result(&resp))
        }
        SessionCommand::Refresh { profile } => {
            let client = build_broker_client(home, data_root, profile, false)?;
            let resp = client
                .request_async(
                    "session.refresh",
                    json!({}),
                    CommandSemantics::Control,
                    None,
                    true,
                )
                .await?;
            Ok(broker_result(&resp))
        }
        SessionCommand::Login { profile, timeout } => {
            run_interactive_login(home, data_root, profile, *timeout).await
        }
        SessionCommand::Close { profile } => {
            let outcome = stop_broker(home, data_root, profile).await?;
            if outcome.get("was_running").and_then(Value::as_bool) == Some(true)
                && outcome.get("stopped").and_then(Value::as_bool) != Some(true)
            {
                return Err(DoudouError::new(
                    "BROKER_STOP_TIMEOUT",
                    "The Profile broker did not release its process and lock in time.",
                )
                .with_details(json!({ "profile_id": profile })));
            }
            Ok(outcome)
        }
    }
}

async fn handle_broker(home: &Path, data_root: &Path, command: &BrokerCommand) -> Result<Value> {
    match command {
        BrokerCommand::List => {
            let brokers = list_brokers(home)?;
            Ok(json!({
                "status": "ok",
                "count": brokers.len(),
                "brokers": brokers,
            }))
        }
        BrokerCommand::Status { profile } => broker_status(home, data_root, profile).await,
        BrokerCommand::Ping { profile } => {
            let client = build_broker_client(home, data_root, profile, false)?;
            let resp = client
                .request_async("ping", json!({}), CommandSemantics::Read, None, true)
                .await?;
            Ok(broker_result(&resp))
        }
        BrokerCommand::Stop { profile } => {
            let outcome = stop_broker(home, data_root, profile).await?;
            if outcome.get("was_running").and_then(Value::as_bool) == Some(true)
                && outcome.get("stopped").and_then(Value::as_bool) != Some(true)
            {
                return Err(DoudouError::new(
                    "BROKER_STOP_TIMEOUT",
                    "The Profile broker did not release its process and lock in time.",
                )
                .with_details(json!({ "profile_id": profile })));
            }
            Ok(outcome)
        }
        BrokerCommand::StopAll => stop_all_brokers(home).await,
        BrokerCommand::TestStart {
            profile,
            fake_browser,
        } => {
            let client = build_broker_client(home, data_root, profile, *fake_browser)?;
            let resp = client
                .request_async("status", json!({}), CommandSemantics::Read, None, true)
                .await?;
            Ok(broker_result(&resp))
        }
        BrokerCommand::Serve {
            profile,
            idle_timeout,
            fake_browser,
            chrome_binary,
        } => {
            let clamped = idle_timeout.clamp(1.0, 86_400.0);
            let mut config = BrokerServerConfig::new(
                home.to_path_buf(),
                data_root.to_path_buf(),
                profile.clone(),
            )
            .with_idle_timeout(Duration::from_secs_f64(clamped))
            .with_fake_browser(*fake_browser);
            if let Some(binary) = chrome_binary {
                config = config.with_chrome_binary(binary.clone());
            }
            let server = BrokerServer::start(config, Some(Arc::new(DefaultBrokerHandler))).await?;
            let descriptor = server.descriptor().clone();

            tokio::select! {
                () = server.wait_stopped() => {}
                _ = tokio::signal::ctrl_c() => {
                    server.close().await;
                }
            }

            Ok(json!({
                "status": "stopped",
                "profile_id": descriptor.profile_id,
                "pid": descriptor.pid,
                "port": descriptor.endpoint.port,
            }))
        }
    }
}

async fn handle_conversations(
    home: &Path,
    data_root: &Path,
    command: &ConversationsCommand,
) -> Result<Value> {
    match command {
        ConversationsCommand::Refresh {
            profile,
            kind,
            inbox,
            force,
        } => {
            let client = build_broker_client(home, data_root, profile, false)?;
            let resp = client
                .request_async(
                    "conversations.refresh",
                    json!({
                        "force": force,
                        "kind": kind.as_str(),
                        "inbox": inbox.as_str(),
                    }),
                    CommandSemantics::Read,
                    None,
                    true,
                )
                .await?;
            Ok(broker_result(&resp))
        }
        ConversationsCommand::Resolve { profile, target } => {
            let client = build_broker_client(home, data_root, profile, false)?;
            let resp = client
                .request_async(
                    "conversations.refresh",
                    json!({
                        "force": false,
                        "kind": "all",
                        "inbox": "all",
                    }),
                    CommandSemantics::Read,
                    None,
                    true,
                )
                .await?;
            let result = broker_result(&resp);
            let conversations = result
                .get("conversations")
                .and_then(Value::as_array)
                .map_or(&[][..], Vec::as_slice);
            let resolved = resolve_conversation(conversations, target)?;
            Ok(json!({
                "status": "resolved",
                "conversation": resolved,
                "refreshed_at": result.get("refreshed_at").cloned().unwrap_or(Value::Null),
            }))
        }
        ConversationsCommand::Select {
            profile,
            selection,
            refreshed_at,
        } => {
            let client = build_broker_client(home, data_root, profile, false)?;
            let resp = client
                .request_async(
                    "conversations.refresh",
                    json!({
                        "force": false,
                        "kind": "all",
                        "inbox": "all",
                    }),
                    CommandSemantics::Read,
                    None,
                    true,
                )
                .await?;
            let result = broker_result(&resp);
            if result.get("refreshed_at").and_then(Value::as_str) != Some(refreshed_at.as_str()) {
                return Err(DoudouError::new(
                    "CONVERSATION_SNAPSHOT_DRIFTED",
                    "The numbered conversation list has changed.",
                ));
            }
            let conversations = result
                .get("conversations")
                .and_then(Value::as_array)
                .map_or(&[][..], Vec::as_slice);
            let count = result
                .get("count")
                .and_then(Value::as_u64)
                .map_or(conversations.len(), |v| v as usize);
            let indices = parse_selection(selection, count)?;
            let selected: Vec<Value> = indices
                .into_iter()
                .filter_map(|index| conversations.get(index.saturating_sub(1)).cloned())
                .collect();
            Ok(json!({
                "status": "selected",
                "refreshed_at": result.get("refreshed_at").cloned().unwrap_or(Value::Null),
                "conversations": selected,
            }))
        }
    }
}

async fn handle_messages(
    home: &Path,
    data_root: &Path,
    command: &MessagesCommand,
) -> Result<Value> {
    match command {
        MessagesCommand::Collect {
            profile,
            conversation,
            from_time,
            to_time,
            max_pages,
            skip_images,
        } => {
            let client = build_broker_client(home, data_root, profile, false)?;
            let resp = client
                .request_async(
                    "messages.collect",
                    json!({
                        "target": conversation,
                        "from": from_time,
                        "to": to_time,
                        "max_pages": max_pages,
                        "skip_images": skip_images,
                    }),
                    CommandSemantics::Read,
                    None,
                    true,
                )
                .await?;
            Ok(broker_result(&resp))
        }
    }
}

async fn handle_send(home: &Path, data_root: &Path, args: &SendArgs) -> Result<Value> {
    let mentions = parse_mentions(&args.at);
    if !mentions.is_empty() {
        return Err(DoudouError::new(
            "MENTION_UNSUPPORTED",
            "Group mentions are not supported.",
        ));
    }
    let client = build_broker_client(home, data_root, &args.profile, false)?;
    if args.confirm {
        let plan_id = require_plan_id(args.plan_id.as_deref())?;
        let resp = client
            .request_async(
                "chat.send.execute",
                json!({ "plan_id": plan_id }),
                CommandSemantics::Write,
                None,
                true,
            )
            .await?;
        return Ok(broker_result(&resp));
    }
    let target = require_target(
        args.conversation_id.as_deref(),
        args.recipient.as_deref(),
        None,
    )?;
    let image = args
        .image
        .as_deref()
        .map(|p| absolutize_path(p).display().to_string());
    let resp = client
        .request_async(
            "chat.send.preview",
            json!({
                "target": target,
                "text": args.text,
                "image": image,
                "reply": args.reply,
                "at": [],
            }),
            CommandSemantics::Read,
            None,
            true,
        )
        .await?;
    Ok(broker_result(&resp))
}

async fn handle_send_batch(home: &Path, data_root: &Path, args: &SendBatchArgs) -> Result<Value> {
    let client = build_broker_client(home, data_root, &args.profile, false)?;
    if args.confirm {
        let plan_id = require_plan_id(args.plan_id.as_deref())?;
        let resp = client
            .request_async(
                "chat.send.batch.execute",
                json!({ "plan_id": plan_id }),
                CommandSemantics::Write,
                None,
                true,
            )
            .await?;
        return Ok(broker_result(&resp));
    }
    let raw_targets = args
        .conversation_ids
        .as_deref()
        .or(args.recipients.as_deref())
        .unwrap_or("");
    let targets: Vec<String> = raw_targets
        .split(',')
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .map(ToOwned::to_owned)
        .collect();
    let resp = client
        .request_async(
            "chat.send.batch.preview",
            json!({
                "targets": targets,
                "text": args.text,
            }),
            CommandSemantics::Read,
            None,
            true,
        )
        .await?;
    Ok(broker_result(&resp))
}

async fn handle_reply(home: &Path, data_root: &Path, args: &ReplyArgs) -> Result<Value> {
    let client = build_broker_client(home, data_root, &args.profile, false)?;
    if args.confirm {
        let plan_id = require_plan_id(args.plan_id.as_deref())?;
        let resp = client
            .request_async(
                "chat.send.execute",
                json!({ "plan_id": plan_id }),
                CommandSemantics::Write,
                None,
                true,
            )
            .await?;
        return Ok(broker_result(&resp));
    }
    let target = require_target(
        args.conversation_id.as_deref(),
        args.recipient.as_deref(),
        None,
    )?;
    let resp = client
        .request_async(
            "chat.send.preview",
            json!({
                "target": target,
                "text": args.text,
                "reply": args.message,
                "at": [],
            }),
            CommandSemantics::Read,
            None,
            true,
        )
        .await?;
    Ok(broker_result(&resp))
}

async fn handle_protocol(
    home: &Path,
    data_root: &Path,
    command: &ProtocolCommand,
) -> Result<Value> {
    match command {
        ProtocolCommand::Send(args) => handle_send(home, data_root, args).await,
        ProtocolCommand::SendBatch(args) => handle_send_batch(home, data_root, args).await,
        ProtocolCommand::Reply(args) => handle_reply(home, data_root, args).await,
        ProtocolCommand::Capture { profile, timeout } => {
            let client = build_broker_client(home, data_root, profile, false)?;
            let resp = client
                .request_async(
                    "protocol.capture",
                    json!({ "timeout_seconds": timeout }),
                    CommandSemantics::Control,
                    None,
                    true,
                )
                .await?;
            Ok(broker_result(&resp))
        }
    }
}

async fn handle_chat(home: &Path, data_root: &Path, command: &ChatCommand) -> Result<Value> {
    match command {
        ChatCommand::Like {
            profile,
            conversation,
            message,
            confirm,
            plan_id,
        } => {
            let client = build_broker_client(home, data_root, profile, false)?;
            if *confirm {
                let id = require_plan_id(plan_id.as_deref())?;
                let resp = client
                    .request_async(
                        "chat.like.execute",
                        json!({ "plan_id": id }),
                        CommandSemantics::Write,
                        None,
                        true,
                    )
                    .await?;
                return Ok(broker_result(&resp));
            }
            let resp = client
                .request_async(
                    "chat.like.preview",
                    json!({
                        "target": conversation,
                        "message": message,
                    }),
                    CommandSemantics::Read,
                    None,
                    true,
                )
                .await?;
            Ok(broker_result(&resp))
        }
        ChatCommand::Send(args) => handle_send(home, data_root, args).await,
        ChatCommand::Reply(args) => handle_reply(home, data_root, args).await,
        ChatCommand::SendBatch(args) => handle_send_batch(home, data_root, args).await,
        ChatCommand::Diagnose(args) => handle_page_doctor(home, data_root, args).await,
    }
}

async fn handle_identity(
    home: &Path,
    data_root: &Path,
    command: &IdentityCommand,
) -> Result<Value> {
    match command {
        IdentityCommand::Resolve { profile, douyin_id } => {
            assert_supported_identity_records(home, data_root, profile)?;
            initialize_roots(home, data_root)?;
            let client = build_broker_client(home, data_root, profile, false)?;
            let resp = client
                .request_async(
                    "identity.resolve",
                    json!({ "douyin_id": douyin_id }),
                    CommandSemantics::Read,
                    None,
                    true,
                )
                .await?;
            Ok(broker_result(&resp))
        }
    }
}

async fn handle_videos(home: &Path, data_root: &Path, command: &VideosCommand) -> Result<Value> {
    match command {
        VideosCommand::Collect {
            profile,
            douyin_id,
            max_pages,
        } => {
            let client = build_broker_client(home, data_root, profile, false)?;
            let resp = client
                .request_async(
                    "videos.collect",
                    json!({
                        "douyin_id": douyin_id,
                        "max_pages": max_pages,
                    }),
                    CommandSemantics::Read,
                    None,
                    true,
                )
                .await?;
            Ok(broker_result(&resp))
        }
    }
}

async fn handle_video(home: &Path, data_root: &Path, command: &VideoCommand) -> Result<Value> {
    match command {
        VideoCommand::Collect {
            profile,
            sec_uid,
            aweme_id,
            max_pages,
        } => {
            let client = build_broker_client(home, data_root, profile, false)?;
            let resp = client
                .request_async(
                    "video.collect",
                    json!({
                        "sec_uid": sec_uid,
                        "aweme_id": aweme_id,
                        "max_pages": max_pages,
                    }),
                    CommandSemantics::Read,
                    None,
                    true,
                )
                .await?;
            Ok(broker_result(&resp))
        }
        VideoCommand::Comment {
            profile,
            sec_uid,
            aweme_id,
            text,
            confirm,
            plan_id,
        } => {
            let client = build_broker_client(home, data_root, profile, false)?;
            if *confirm {
                let id = require_plan_id(plan_id.as_deref())?;
                let resp = client
                    .request_async(
                        "video.interaction.execute",
                        json!({
                            "operation": "video.comment.create",
                            "plan_id": id,
                        }),
                        CommandSemantics::Write,
                        None,
                        true,
                    )
                    .await?;
                return Ok(broker_result(&resp));
            }
            let resp = client
                .request_async(
                    "video.interaction.preview",
                    json!({
                        "operation": "video.comment.create",
                        "payload": {
                            "sec_uid": sec_uid,
                            "aweme_id": aweme_id,
                            "text": text,
                        },
                    }),
                    CommandSemantics::Read,
                    None,
                    true,
                )
                .await?;
            Ok(broker_result(&resp))
        }
        VideoCommand::Reply {
            profile,
            sec_uid,
            aweme_id,
            text,
            comment_id,
            comment_text,
            confirm,
            plan_id,
        } => {
            let client = build_broker_client(home, data_root, profile, false)?;
            if *confirm {
                let id = require_plan_id(plan_id.as_deref())?;
                let resp = client
                    .request_async(
                        "video.interaction.execute",
                        json!({
                            "operation": "video.comment.reply",
                            "plan_id": id,
                        }),
                        CommandSemantics::Write,
                        None,
                        true,
                    )
                    .await?;
                return Ok(broker_result(&resp));
            }
            let resp = client
                .request_async(
                    "video.interaction.preview",
                    json!({
                        "operation": "video.comment.reply",
                        "payload": {
                            "sec_uid": sec_uid,
                            "aweme_id": aweme_id,
                            "text": text,
                            "comment": {
                                "cid": comment_id,
                                "text": comment_text,
                                "digg_count": 0,
                                "user_digged": false,
                            },
                        },
                    }),
                    CommandSemantics::Read,
                    None,
                    true,
                )
                .await?;
            Ok(broker_result(&resp))
        }
        VideoCommand::Like {
            profile,
            sec_uid,
            aweme_id,
            comment_id,
            comment_text,
            digg_count,
            already_liked,
            confirm,
            plan_id,
        } => {
            let client = build_broker_client(home, data_root, profile, false)?;
            if *confirm {
                let id = require_plan_id(plan_id.as_deref())?;
                let resp = client
                    .request_async(
                        "video.interaction.execute",
                        json!({
                            "operation": "video.comment.like",
                            "plan_id": id,
                        }),
                        CommandSemantics::Write,
                        None,
                        true,
                    )
                    .await?;
                return Ok(broker_result(&resp));
            }
            let resp = client
                .request_async(
                    "video.interaction.preview",
                    json!({
                        "operation": "video.comment.like",
                        "payload": {
                            "sec_uid": sec_uid,
                            "aweme_id": aweme_id,
                            "comment": {
                                "cid": comment_id,
                                "text": comment_text,
                                "digg_count": digg_count,
                                "user_digged": already_liked,
                            },
                        },
                    }),
                    CommandSemantics::Read,
                    None,
                    true,
                )
                .await?;
            Ok(broker_result(&resp))
        }
    }
}

async fn handle_page_doctor(home: &Path, data_root: &Path, args: &PageDoctorArgs) -> Result<Value> {
    let client = build_broker_client(home, data_root, &args.profile, false)?;
    let baseline = args
        .baseline
        .as_deref()
        .map(|p| absolutize_path(p).display().to_string());
    let resp = client
        .request_async(
            "page.doctor",
            json!({
                "conversation": args.conversation,
                "baseline": baseline,
            }),
            CommandSemantics::Read,
            None,
            true,
        )
        .await?;
    Ok(broker_result(&resp))
}
