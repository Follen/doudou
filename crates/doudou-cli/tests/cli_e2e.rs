//! End-to-end and parser-contract integration tests for `doudou-cli`.
//!
//! Covers:
//! - Ported assertions from `docs/verification/tests/test_cli_doctor.py` and `test_web_plans_cli.py`
//! - `cf`-style `cli search` and `cli schema` live `clap::Command` introspection
//! - OS `AppData` (`--home`) vs project `--data-root` separation in `profile` CRUD
//! - Offline `doudou doctor` NDJSON output contract
//! - Real subprocess `doudou broker serve` auto-launch via `BrokerLauncher` (`--fake-browser`)

use std::fs;
use std::path::Path;
use std::process::{Command, Output};

use clap::Parser as _;
use doudou_cli::{Cli, Commands, DoctorArgs, SessionCommand, dispatch};
use doudou_core::{CURRENT_PROFILE_RUNTIME, CURRENT_PROFILE_SCHEMA, read_json};
use doudou_storage::{CHROME_CONFIG_SCHEMA, create_profile};
use serde_json::{Value, json};
use tempfile::TempDir;

fn doudou_bin() -> &'static str {
    env!("CARGO_BIN_EXE_doudou")
}

fn run_doudou(args: &[&str]) -> Output {
    Command::new(doudou_bin())
        .args(args)
        .output()
        .expect("failed executing doudou binary")
}

fn parse_ndjson(stdout: &[u8]) -> Vec<Value> {
    let text = String::from_utf8_lossy(stdout);
    text.lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| {
            serde_json::from_str::<Value>(line)
                .unwrap_or_else(|err| panic!("invalid NDJSON line `{line}`: {err}"))
        })
        .collect()
}

fn home_data_args(home: &Path, data_root: &Path) -> [String; 4] {
    [
        "--home".to_owned(),
        home.display().to_string(),
        "--data-root".to_owned(),
        data_root.display().to_string(),
    ]
}

// ---------------------------------------------------------------------------
// Ported `test_cli_doctor.py` & `test_web_plans_cli.py` assertions
// ---------------------------------------------------------------------------

#[test]
fn doctor_does_not_launch_browser_by_default_and_launch_probe_is_explicit() {
    let default_cli = Cli::try_parse_from(["doudou", "--data-root", "data", "doctor"])
        .expect("default doctor must parse");
    match default_cli.command {
        Commands::Doctor(DoctorArgs { launch_probe, .. }) => {
            assert!(!launch_probe, "doctor must not launch Chrome by default");
        }
        other => panic!("unexpected command: {other:?}"),
    }

    let probe_cli =
        Cli::try_parse_from(["doudou", "--data-root", "data", "doctor", "--launch-probe"])
            .expect("doctor --launch-probe must parse");
    match probe_cli.command {
        Commands::Doctor(DoctorArgs { launch_probe, .. }) => {
            assert!(launch_probe, "doctor --launch-probe must set launch_probe");
        }
        other => panic!("unexpected command: {other:?}"),
    }

    // Mutually exclusive with hidden legacy --skip-launch flag
    assert!(
        Cli::try_parse_from(["doudou", "doctor", "--launch-probe", "--skip-launch"]).is_err(),
        "--launch-probe and --skip-launch must conflict"
    );
}

#[tokio::test]
async fn doctor_rejects_legacy_profile_casing_before_probe() {
    let dir = TempDir::new().expect("tempdir");
    let home = dir.path().join("appdata");
    let data_root = dir.path().join("project");
    fs::create_dir_all(data_root.join("Profile")).expect("create legacy Profile/");

    let cli = Cli::try_parse_from([
        "doudou",
        "--home",
        &home.display().to_string(),
        "--data-root",
        &data_root.display().to_string(),
        "doctor",
        "--launch-probe",
    ])
    .expect("parse doctor");

    let err = dispatch(&cli)
        .await
        .expect_err("legacy Profile/ root must fail before probing");
    assert_eq!(err.code, "LEGACY_UNSUPPORTED");
}

#[tokio::test]
async fn identity_legacy_record_fails_before_broker_start() {
    let dir = TempDir::new().expect("tempdir");
    let home = dir.path().join("appdata");
    let data_root = dir.path().join("project");
    let record_file = data_root
        .join("message")
        .join("profiles")
        .join("profile-a")
        .join("conversations")
        .join("fixture")
        .join("messages.jsonl");
    fs::create_dir_all(record_file.parent().expect("parent")).expect("mkdirs");
    fs::write(
        &record_file,
        format!("{}\n", json!({"sender": {"uid": "1", "sec_uid": "SEC"}})),
    )
    .expect("write legacy record");

    let cli = Cli::try_parse_from([
        "doudou",
        "--home",
        &home.display().to_string(),
        "--data-root",
        &data_root.display().to_string(),
        "identity",
        "resolve",
        "--profile",
        "profile-a",
        "--douyin-id",
        "target",
    ])
    .expect("parse identity resolve");

    let err = dispatch(&cli)
        .await
        .expect_err("legacy identity record must fail before broker start");
    assert_eq!(err.code, "LEGACY_UNSUPPORTED");
}

#[test]
fn identity_migrate_and_warmup_are_not_cli_commands_and_session_refresh_exists() {
    assert!(
        Cli::try_parse_from(["doudou", "identity", "migrate", "--profile", "profile-a"]).is_err(),
        "`identity migrate` must not exist"
    );
    assert!(
        Cli::try_parse_from(["doudou", "warmup", "--profile", "work"]).is_err(),
        "`warmup` must not exist"
    );
    let parsed = Cli::try_parse_from(["doudou", "session", "refresh", "--profile", "work"])
        .expect("`session refresh` must parse");
    match parsed.command {
        Commands::Session {
            command: SessionCommand::Refresh { profile },
        } => assert_eq!(profile, "work"),
        other => panic!("unexpected command: {other:?}"),
    }
}

#[tokio::test]
async fn legacy_at_mention_fails_before_broker_start() {
    let dir = TempDir::new().expect("tempdir");
    let home = dir.path().join("appdata");
    let data_root = dir.path().join("project");

    for sub in ["protocol", "chat"] {
        let cli = Cli::try_parse_from([
            "doudou",
            "--home",
            &home.display().to_string(),
            "--data-root",
            &data_root.display().to_string(),
            sub,
            "send",
            "--profile",
            "profile-a",
            "--recipient",
            "group-a",
            "--text",
            "hello",
            "--at",
            "member-a",
        ])
        .expect("parse send with --at");

        let err = dispatch(&cli)
            .await
            .expect_err("--at must fail closed with MENTION_UNSUPPORTED");
        assert_eq!(err.code, "MENTION_UNSUPPORTED");
    }
}

#[tokio::test]
async fn confirm_without_plan_id_fails_with_plan_id_required() {
    let dir = TempDir::new().expect("tempdir");
    let home = dir.path().join("appdata");
    let data_root = dir.path().join("project");
    create_profile(&home, &data_root, "work", None).expect("create profile");

    let cases: [&[&str]; 6] = [
        &["protocol", "send", "--profile", "work", "--confirm"],
        &["protocol", "reply", "--profile", "work", "--confirm"],
        &["protocol", "send-batch", "--profile", "work", "--confirm"],
        &["chat", "like", "--profile", "work", "--confirm"],
        &["video", "comment", "--profile", "work", "--confirm"],
        &["video", "like", "--profile", "work", "--confirm"],
    ];

    for case in cases {
        let mut argv = vec![
            "doudou".to_owned(),
            "--home".to_owned(),
            home.display().to_string(),
            "--data-root".to_owned(),
            data_root.display().to_string(),
        ];
        argv.extend(case.iter().map(|s| (*s).to_owned()));
        let cli = Cli::try_parse_from(&argv).expect("parse confirm case");
        let err = dispatch(&cli)
            .await
            .expect_err("--confirm without --plan-id must fail");
        assert_eq!(err.code, "PLAN_ID_REQUIRED", "case {case:?}");
    }
}

// ---------------------------------------------------------------------------
// Subprocess binary E2E tests: --version, --help, cli search, cli schema
// ---------------------------------------------------------------------------

#[test]
fn binary_version_and_help_succeed() {
    let version = run_doudou(&["--version"]);
    assert!(version.status.success(), "--version must exit 0");
    let ver_stdout = String::from_utf8_lossy(&version.stdout);
    assert!(
        ver_stdout.contains(env!("CARGO_PKG_VERSION")),
        "expected package version in `{ver_stdout}`"
    );

    let help = run_doudou(&["--help"]);
    assert!(help.status.success(), "--help must exit 0");
    let help_stdout = String::from_utf8_lossy(&help.stdout);
    for expected in [
        "doctor",
        "profile",
        "session",
        "broker",
        "conversations",
        "messages",
        "protocol",
        "chat",
        "identity",
        "videos",
        "video",
        "page-doctor",
        "cli",
        "--home",
        "--data-root",
    ] {
        assert!(
            help_stdout.contains(expected),
            "`--help` missing `{expected}`:\n{help_stdout}"
        );
    }
}

#[test]
fn cli_search_finds_commands_and_error_codes_by_intent() {
    // Text search for "send"
    let send_text = run_doudou(&["cli", "search", "send"]);
    assert!(send_text.status.success());
    let stdout = String::from_utf8_lossy(&send_text.stdout);
    assert!(stdout.contains("protocol send"), "stdout:\n{stdout}");

    // JSON search for "like"
    let like_json = run_doudou(&["cli", "search", "like", "--json"]);
    assert!(like_json.status.success());
    let payload: Value =
        serde_json::from_slice(&like_json.stdout).expect("valid JSON from cli search --json");
    let paths: Vec<&str> = payload["matches"]
        .as_array()
        .expect("matches array")
        .iter()
        .filter_map(|m| m["path"].as_str())
        .collect();
    assert!(paths.contains(&"chat like"), "paths: {paths:?}");
    assert!(paths.contains(&"video like"), "paths: {paths:?}");

    // Intent search for "stranger" finds conversations refresh / messages collect
    let stranger_json = run_doudou(&["cli", "search", "stranger", "--json"]);
    assert!(stranger_json.status.success());
    let stranger: Value = serde_json::from_slice(&stranger_json.stdout).expect("valid JSON");
    let stranger_paths: Vec<&str> = stranger["matches"]
        .as_array()
        .expect("matches")
        .iter()
        .filter_map(|m| m["path"].as_str())
        .collect();
    assert!(
        stranger_paths.contains(&"conversations refresh"),
        "expected conversations refresh in {stranger_paths:?}"
    );

    // Intent search for "appdata" finds profile/doctor commands
    let appdata_json = run_doudou(&["cli", "search", "appdata", "--json"]);
    assert!(appdata_json.status.success());
    let appdata: Value = serde_json::from_slice(&appdata_json.stdout).expect("valid JSON");
    assert!(
        appdata["total_matches"].as_u64().unwrap_or(0) >= 2,
        "expected appdata matches: {appdata}"
    );

    // Searching by error code surfaces matched_error_codes
    let err_json = run_doudou(&["cli", "search", "WRITE_RESULT_UNKNOWN", "--json"]);
    assert!(err_json.status.success());
    let err_payload: Value = serde_json::from_slice(&err_json.stdout).expect("valid JSON");
    let matched_codes: Vec<&str> = err_payload["matched_error_codes"]
        .as_array()
        .expect("matched_error_codes")
        .iter()
        .filter_map(|c| c["code"].as_str())
        .collect();
    assert!(
        matched_codes.contains(&"WRITE_RESULT_UNKNOWN"),
        "matched_codes: {matched_codes:?}"
    );
}

#[test]
fn cli_schema_emits_full_and_scoped_machine_readable_json() {
    // Full schema
    let full = run_doudou(&["cli", "schema"]);
    assert!(full.status.success());
    let full_json: Value = serde_json::from_slice(&full.stdout).expect("valid schema JSON");
    assert_eq!(full_json["schema"], "doudou.cli-schema.v1");
    assert_eq!(full_json["name"], "doudou");
    assert!(
        full_json["commands"]
            .as_array()
            .is_some_and(|cmds| cmds.len() >= 12),
        "expected full command tree"
    );
    assert!(
        full_json["error_codes"]
            .as_array()
            .is_some_and(|codes| codes.len() >= 20),
        "expected categorized error code catalog"
    );

    // Scoped to `profile create`
    let scoped = run_doudou(&["cli", "schema", "profile", "create"]);
    assert!(scoped.status.success());
    let scoped_json: Value = serde_json::from_slice(&scoped.stdout).expect("valid scoped JSON");
    assert_eq!(scoped_json["target_command"], "profile create");
    let arg_longs: Vec<&str> = scoped_json["command"]["args"]
        .as_array()
        .expect("args")
        .iter()
        .filter_map(|a| a["long"].as_str())
        .collect();
    assert!(arg_longs.contains(&"profile"), "args: {arg_longs:?}");
    assert!(arg_longs.contains(&"display-name"), "args: {arg_longs:?}");

    // Unknown command path fails cleanly with NDJSON error
    let bad = run_doudou(&["cli", "schema", "nonexistent-command"]);
    assert!(!bad.status.success());
    let events = parse_ndjson(&bad.stdout);
    assert_eq!(
        events.last().expect("error event")["code"],
        "UNKNOWN_COMMAND"
    );
}

// ---------------------------------------------------------------------------
// Profile lifecycle (OS AppData --home vs project --data-root) & offline doctor
// ---------------------------------------------------------------------------

#[test]
fn profile_create_list_status_delete_respects_home_and_data_root_separation() {
    let dir = TempDir::new().expect("tempdir");
    let home = dir.path().join("appdata");
    let data_root = dir.path().join("project");
    let base = home_data_args(&home, &data_root);

    // 1. Create profile
    let create = run_doudou(&[
        &base[0],
        &base[1],
        &base[2],
        &base[3],
        "profile",
        "create",
        "--profile",
        "work",
        "--display-name",
        "Work Account",
    ]);
    assert!(
        create.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&create.stderr)
    );
    let create_events = parse_ndjson(&create.stdout);
    assert_eq!(create_events[0]["event"], "started");
    assert_eq!(create_events[0]["command"], "profile");
    let create_res = &create_events[1]["result"];
    assert_eq!(create_res["status"], "created");
    assert_eq!(create_res["profile"]["schema"], CURRENT_PROFILE_SCHEMA);
    assert_eq!(create_res["profile"]["runtime"], CURRENT_PROFILE_RUNTIME);

    let profile_dir = home.join("profiles").join("work");
    let msg_dir = data_root.join("message").join("profiles").join("work");
    assert!(profile_dir.join("profile.json").is_file());
    assert!(profile_dir.join("chrome-config.json").is_file());
    assert!(profile_dir.join("browser-data").is_dir());
    assert!(msg_dir.is_dir());

    let chrome_cfg = read_json(&profile_dir.join("chrome-config.json"), false)
        .expect("read chrome-config")
        .expect("chrome-config present");
    assert_eq!(chrome_cfg["schema"], CHROME_CONFIG_SCHEMA);

    // 2. List profiles
    let list = run_doudou(&[&base[0], &base[1], &base[2], &base[3], "profile", "list"]);
    assert!(list.status.success());
    let list_events = parse_ndjson(&list.stdout);
    let list_res = &list_events[1]["result"];
    assert_eq!(list_res["status"], "ok");
    assert_eq!(list_res["count"], 1);
    assert_eq!(list_res["profiles"][0]["profile_id"], "work");
    assert_eq!(list_res["profiles"][0]["lock_status"], "free");

    // 3. Profile status (`show` alias)
    let status = run_doudou(&[
        &base[0],
        &base[1],
        &base[2],
        &base[3],
        "profile",
        "show",
        "--profile",
        "work",
    ]);
    assert!(status.status.success());
    let status_events = parse_ndjson(&status.stdout);
    assert_eq!(status_events[1]["result"]["profile"]["profile_id"], "work");
    assert!(status_events[1]["result"]["broker"].is_null());

    // 4. Delete profile + messages
    let delete = run_doudou(&[
        &base[0],
        &base[1],
        &base[2],
        &base[3],
        "profile",
        "delete",
        "--profile",
        "work",
        "--with-messages",
    ]);
    assert!(delete.status.success());
    let del_events = parse_ndjson(&delete.stdout);
    assert_eq!(del_events[1]["result"]["status"], "deleted");
    assert_eq!(del_events[1]["result"]["deleted_profile_dir"], true);
    assert_eq!(del_events[1]["result"]["deleted_message_dir"], true);
    assert!(!profile_dir.exists());
    assert!(!msg_dir.exists());
}

#[test]
fn doctor_offline_returns_structured_ndjson_with_status_and_chrome_discovery() {
    let dir = TempDir::new().expect("tempdir");
    let home = dir.path().join("appdata");
    let data_root = dir.path().join("project");
    let base = home_data_args(&home, &data_root);

    let out = run_doudou(&[&base[0], &base[1], &base[2], &base[3], "doctor"]);
    assert!(
        out.status.success(),
        "doctor failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let events = parse_ndjson(&out.stdout);
    assert_eq!(
        events.len(),
        2,
        "expected started + result events: {events:?}"
    );
    assert_eq!(events[0]["event"], "started");
    assert_eq!(events[0]["command"], "doctor");
    let result = &events[1]["result"];
    assert_eq!(result["schema"], "doudou.doctor.v2");
    assert!(matches!(result["status"].as_str(), Some("ok" | "degraded")));
    assert_eq!(result["home"]["writable"], true);
    assert_eq!(result["data_root"]["writable"], true);
    assert!(result["chrome"].is_object());
    assert!(result["launch_probe"].is_null());
}

#[test]
fn broker_launcher_spawns_serve_subprocess_and_handles_fake_browser_rpc() {
    let dir = TempDir::new().expect("tempdir");
    let home = dir.path().join("appdata");
    let data_root = dir.path().join("project");
    let base = home_data_args(&home, &data_root);

    let create = run_doudou(&[
        &base[0],
        &base[1],
        &base[2],
        &base[3],
        "profile",
        "create",
        "--profile",
        "fake-e2e",
    ]);
    assert!(create.status.success());

    // Spawns detached `doudou broker serve --home ... --data-root ... --profile fake-e2e --fake-browser`
    let started = run_doudou(&[
        &base[0],
        &base[1],
        &base[2],
        &base[3],
        "broker",
        "test-start",
        "--profile",
        "fake-e2e",
        "--fake-browser",
    ]);
    assert!(
        started.status.success(),
        "broker test-start failed: stdout={}, stderr={}",
        String::from_utf8_lossy(&started.stdout),
        String::from_utf8_lossy(&started.stderr)
    );
    let start_events = parse_ndjson(&started.stdout);
    assert_eq!(start_events[1]["result"]["profile_id"], "fake-e2e");

    // Verify broker list & ping & session status against live spawned broker
    let list = run_doudou(&[&base[0], &base[1], &base[2], &base[3], "broker", "list"]);
    assert!(list.status.success());
    let list_events = parse_ndjson(&list.stdout);
    assert_eq!(list_events[1]["result"]["count"], 1);

    let ping = run_doudou(&[
        &base[0],
        &base[1],
        &base[2],
        &base[3],
        "broker",
        "ping",
        "--profile",
        "fake-e2e",
    ]);
    assert!(ping.status.success());
    let ping_events = parse_ndjson(&ping.stdout);
    assert_eq!(ping_events[1]["result"]["pong"], true);

    let session = run_doudou(&[
        &base[0],
        &base[1],
        &base[2],
        &base[3],
        "session",
        "status",
        "--profile",
        "fake-e2e",
    ]);
    assert!(session.status.success());
    let session_events = parse_ndjson(&session.stdout);
    assert_eq!(session_events[1]["result"]["status"], "ok");

    // Cleanly stop the spawned broker so temp dir removes cleanly
    let stop = run_doudou(&[
        &base[0],
        &base[1],
        &base[2],
        &base[3],
        "broker",
        "stop",
        "--profile",
        "fake-e2e",
    ]);
    assert!(stop.status.success());
    let stop_events = parse_ndjson(&stop.stdout);
    assert_eq!(stop_events[1]["result"]["stopped"], true);
}

#[test]
fn data_root_defaults_to_home_and_honors_env_and_cli_overrides() {
    let dir = TempDir::new().expect("tempdir");
    let cwd = dir.path().join("clean-cwd");
    let home = dir.path().join("appdata");
    let env_data = dir.path().join("env-data");
    let cli_data = dir.path().join("cli-data");
    fs::create_dir_all(&cwd).expect("mkdir cwd");

    // 1. Only --home passed: data_root defaults to home and CWD stays clean
    let default_out = Command::new(doudou_bin())
        .current_dir(&cwd)
        .env_remove("DOUDOU_DATA_ROOT")
        .env_remove("DOUDOU_HOME")
        .args([
            "--home",
            &home.display().to_string(),
            "profile",
            "create",
            "--profile",
            "default-root",
        ])
        .output()
        .expect("run default-root profile create");
    assert!(
        default_out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&default_out.stderr)
    );
    let default_events = parse_ndjson(&default_out.stdout);
    assert_eq!(
        default_events[0]["data_root"], default_events[0]["home"],
        "data_root must default to home: {:?}",
        default_events[0]
    );
    assert!(
        home.join("message")
            .join("profiles")
            .join("default-root")
            .is_dir()
    );
    assert!(
        !cwd.join("message").exists(),
        "CWD must not be polluted with message/"
    );

    // 2. DOUDOU_DATA_ROOT overrides default home
    let env_out = Command::new(doudou_bin())
        .current_dir(&cwd)
        .env("DOUDOU_HOME", &home)
        .env("DOUDOU_DATA_ROOT", &env_data)
        .args(["doctor"])
        .output()
        .expect("run doctor with DOUDOU_DATA_ROOT");
    assert!(env_out.status.success());
    let env_events = parse_ndjson(&env_out.stdout);
    assert_eq!(
        fs::canonicalize(Path::new(
            env_events[0]["data_root"].as_str().expect("data_root str")
        ))
        .expect("canonical actual env_data"),
        fs::canonicalize(&env_data).expect("canonical expected env_data")
    );
    assert!(env_data.join("message").is_dir());

    // 3. Explicit --data-root beats DOUDOU_DATA_ROOT
    let cli_out = Command::new(doudou_bin())
        .current_dir(&cwd)
        .env("DOUDOU_HOME", &home)
        .env("DOUDOU_DATA_ROOT", &env_data)
        .args(["--data-root", &cli_data.display().to_string(), "doctor"])
        .output()
        .expect("run doctor with --data-root");
    assert!(cli_out.status.success());
    let cli_events = parse_ndjson(&cli_out.stdout);
    assert_eq!(
        fs::canonicalize(Path::new(
            cli_events[0]["data_root"].as_str().expect("data_root str")
        ))
        .expect("canonical actual cli_data"),
        fs::canonicalize(&cli_data).expect("canonical expected cli_data")
    );
    assert!(cli_data.join("message").is_dir());
}
