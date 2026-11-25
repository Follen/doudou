//! Process identity, profile lock, runtime home resolution, and `doctor` diagnostics tests.

use doudou_core::{broker_registry_root, doudou_home, profiles_root};
use doudou_runtime::{
    DoctorOptions, PROFILE_LOCK_SCHEMA, ProcessIdentity, ProfileLock, current_process_identity,
    doctor, lock_is_live, parse_windows_process_json, proc_stat_is_zombie, process_identity,
    same_process, windows_process_query_script,
};
use doudou_storage::create_profile;
#[cfg(windows)]
use doudou_storage::platform_background_mode;
use tempfile::TempDir;

#[test]
fn windows_process_identity_pins_utf8_handles_empty_and_parses_unicode_json() {
    let script = windows_process_query_script(123);
    assert!(script.contains("UTF8Encoding]::new($false)"));
    assert!(script.contains("ProcessId = 123"));

    assert!(parse_windows_process_json("").is_none());
    assert!(parse_windows_process_json("   ").is_none());

    let parsed = parse_windows_process_json(
        r#"{"pid":123,"start":"2026-08-04T00:00:00.0000000Z","command":"中文"}"#,
    )
    .expect("utf8 json");
    assert_eq!(parsed.pid, 123);
    assert_eq!(parsed.command.as_deref(), Some("中文"));

    assert!(proc_stat_is_zombie("42 (chrome) Z 1 2"));
    assert!(!proc_stat_is_zombie("42 (chrome) S 1 2"));

    let cur = current_process_identity();
    assert_eq!(cur.pid, std::process::id());
    assert!(same_process(Some(&cur), process_identity(cur.pid).as_ref()));
    assert!(!same_process(
        Some(&cur),
        Some(&ProcessIdentity::new(
            cur.pid,
            Some("2000-01-01T00:00:00Z".to_owned()),
            None
        ))
    ));
}

#[cfg(windows)]
#[test]
fn windows_background_mode_is_true_headless() {
    assert_eq!(platform_background_mode(), "windows_headless");
}

#[test]
fn runtime_home_controls_profiles_and_broker_registry_roots() {
    let dir = TempDir::new().expect("tempdir");
    let configured = dir.path().join("managed");
    let home = doudou_home(Some(&configured));
    assert_eq!(home, configured);
    assert_eq!(profiles_root(&home), configured.join("profiles"));
    assert_eq!(broker_registry_root(&home), configured.join("brokers"));
}

#[tokio::test]
async fn doctor_reports_writable_roots_profiles_brokers_and_chrome_discovery() {
    let dir = TempDir::new().expect("tempdir");
    let home = dir.path().join("appdata");
    let data_root = dir.path().join("project");
    let (_, paths) = create_profile(&home, &data_root, "work", None).expect("profile");

    // Acquire a live profile lock to verify doctor's profile summary reports lock status.
    let mut lock = ProfileLock::new(paths.lock.clone(), "work", data_root.clone(), "broker");
    let owner = lock.acquire().expect("acquire lock");
    assert_eq!(owner.schema, PROFILE_LOCK_SCHEMA);
    assert!(lock_is_live(&paths.lock));

    let report = doctor(&home, &data_root, &DoctorOptions::new())
        .await
        .expect("doctor");
    assert_eq!(report["schema"], "doudou.doctor.v2");
    assert_eq!(report["home"]["writable"], true);
    assert_eq!(report["data_root"]["writable"], true);
    assert_eq!(report["profiles"]["count"], 1);
    assert_eq!(report["profiles"]["items"][0]["profile_id"], "work");
    assert_eq!(report["profiles"]["items"][0]["lock_status"], "in_use");
    assert_eq!(report["brokers"]["count"], 0);
    assert!(report["launch_probe"].is_null());

    lock.release();
    assert!(!lock_is_live(&paths.lock));

    // Explicit missing binary produces degraded report cleanly without panicking.
    let bad_opts = DoctorOptions::new().with_chrome_binary(dir.path().join("missing-chrome.exe"));
    let degraded = doctor(&home, &data_root, &bad_opts)
        .await
        .expect("doctor with missing binary");
    assert_eq!(degraded["ok"], false);
    assert_eq!(degraded["chrome"]["available"], false);
    assert_eq!(degraded["chrome"]["error_code"], "CHROME_NOT_FOUND");
}
