//! Persistent storage for Doudou profiles, chats, content, single-use write plans, and receipts.

pub mod content;
pub mod plans;
pub mod profile;
pub mod receipts;
pub mod repository;

pub use content::{
    merge_video_comments, read_comment_checkpoint, read_identity, read_video_comments,
    read_video_detail, read_video_index, user_directory, video_directory, write_comment_checkpoint,
    write_identity, write_video, write_video_index,
};
pub use plans::{
    DEFAULT_PLAN_TTL_SECONDS, PlanStatus, TerminalPlanStatus, WRITE_PLAN_SCHEMA, WritePlan,
    complete_plan, confirm_plan, create_plan, load_plan, payload_hash,
};
pub use profile::{
    CHROME_CONFIG_SCHEMA, DeleteScope, create_profile, default_browser_config, delete_profile,
    list_profiles, load_profile, platform_background_mode, update_profile,
};
pub use receipts::{ReceiptKind, read_receipt, write_receipt};
pub use repository::{
    conversation_folder, list_runs, merge_messages, read_conflicts, read_conversation_index,
    read_messages, read_run, read_session_index_snapshot, unknown_field_inventory,
    write_conversation_index, write_run, write_session_index_snapshot,
};

#[cfg(test)]
mod tests {
    use std::fs;

    use chrono::DateTime;
    use doudou_core::{
        CURRENT_PROFILE_RUNTIME, CURRENT_PROFILE_SCHEMA, atomic_write_json, read_json,
    };
    use serde_json::{Map, Value, json};
    use tempfile::TempDir;

    use super::*;

    #[test]
    fn profile_crud_uses_appdata_home_chrome_config_and_v3_schema() {
        let dir = TempDir::new().unwrap();
        let home = dir.path().join("appdata");
        let data_root = dir.path().join("project");

        let (metadata, paths) =
            create_profile(&home, &data_root, "工作/账号", Some("工作账号")).unwrap();
        assert_eq!(metadata["schema"], CURRENT_PROFILE_SCHEMA);
        assert_eq!(metadata["runtime"], CURRENT_PROFILE_RUNTIME);
        assert_eq!(metadata["profile_id"], "工作~002F账号");
        assert_eq!(metadata["display_name"], "工作账号");
        assert_eq!(metadata["browser_config"], "chrome-config.json");
        assert!(paths.browser_data.is_dir());
        assert!(paths.browser_config.is_file());
        assert!(paths.message.is_dir());

        let config = read_json(&paths.browser_config, false).unwrap().unwrap();
        assert_eq!(config["schema"], CHROME_CONFIG_SCHEMA);
        assert_eq!(config["profile_id"], "工作~002F账号");
        assert!(config["config"]["canvas:seed"].as_u64().unwrap() >= 1);

        let (loaded, _) = load_profile(&home, &data_root, "工作/账号").unwrap();
        assert_eq!(loaded["display_name"], "工作账号");

        let mut patch = Map::new();
        patch.insert("login_status".to_owned(), json!("valid"));
        patch.insert(
            "last_login_verified_at".to_owned(),
            json!("2026-11-25T10:00:00.000Z"),
        );
        let updated = update_profile(&home, &data_root, "工作/账号", &patch).unwrap();
        assert_eq!(updated["login_status"], "valid");
        assert_eq!(
            updated["last_login_verified_at"],
            "2026-11-25T10:00:00.000Z"
        );

        let dup = create_profile(&home, &data_root, "工作/账号", None).unwrap_err();
        assert_eq!(dup.code, "PROFILE_EXISTS");

        let deleted = delete_profile(
            &home,
            &data_root,
            "工作/账号",
            DeleteScope::ProfileAndMessages,
        )
        .unwrap();
        assert_eq!(deleted["deleted_profile_dir"], true);
        assert_eq!(deleted["deleted_message_dir"], true);
        assert!(!paths.profile.exists());
        assert!(!paths.message.exists());
        assert_eq!(
            load_profile(&home, &data_root, "工作/账号")
                .unwrap_err()
                .code,
            "PROFILE_NOT_FOUND"
        );
    }

    #[test]
    fn profile_rejects_legacy_root_and_legacy_metadata_without_mutation() {
        let dir = TempDir::new().unwrap();
        let home = dir.path().join("appdata");
        let data_root = dir.path().join("project");

        // Legacy Profile/ root casing in project data_root
        let legacy_root = data_root.join("Profile");
        fs::create_dir_all(&legacy_root).unwrap();
        fs::write(legacy_root.join("marker.txt"), "legacy").unwrap();
        let err = create_profile(&home, &data_root, "work", None).unwrap_err();
        assert_eq!(err.code, "LEGACY_UNSUPPORTED");
        assert!(!data_root.join("message").exists());

        // Legacy browser profile metadata under home
        let clean_project = dir.path().join("clean_project");
        let legacy_profile = home.join("profiles").join("legacy");
        fs::create_dir_all(&legacy_profile).unwrap();
        let meta_path = legacy_profile.join("profile.json");
        let original =
            r#"{"schema":"doudou.profile.v2","runtime":"camoufox-python","profile_id":"legacy"}"#;
        fs::write(&meta_path, original).unwrap();

        assert_eq!(
            load_profile(&home, &clean_project, "legacy")
                .unwrap_err()
                .code,
            "LEGACY_UNSUPPORTED"
        );
        assert_eq!(
            create_profile(&home, &clean_project, "legacy", None)
                .unwrap_err()
                .code,
            "LEGACY_UNSUPPORTED"
        );
        assert_eq!(fs::read_to_string(&meta_path).unwrap(), original);
        assert!(!clean_project.join("message").exists());
    }

    #[test]
    fn list_profiles_reports_summaries_lock_probe_and_skips_legacy() {
        let dir = TempDir::new().unwrap();
        let home = dir.path().join("appdata");
        let data_root = dir.path().join("project");

        let (_meta, paths) = create_profile(&home, &data_root, "work", None).unwrap();

        // Coexisting legacy profile should be ignored by list_profiles
        let legacy_dir = home.join("profiles").join("old-camoufox");
        fs::create_dir_all(&legacy_dir).unwrap();
        atomic_write_json(
            &legacy_dir.join("profile.json"),
            &json!({"schema": "doudou.profile.v2", "runtime": "camoufox-python", "profile_id": "old-camoufox"}),
            None,
        )
        .unwrap();

        atomic_write_json(
            &paths.message.join("conversations.json"),
            &json!({"count": 2, "conversations": []}),
            None,
        )
        .unwrap();
        atomic_write_json(
            &paths.session_index.join("latest.json"),
            &json!({"completed_at": "2026-11-25T00:00:00.000Z"}),
            None,
        )
        .unwrap();
        fs::create_dir_all(paths.conversations.join("group__one__1")).unwrap();
        fs::create_dir_all(paths.conversations.join("media")).unwrap();
        fs::create_dir_all(paths.message.join("content").join("users").join("user-1")).unwrap();
        fs::create_dir_all(paths.message.join("content").join("videos").join("video-1")).unwrap();
        atomic_write_json(
            &paths
                .sends
                .join("2026-11-25")
                .join("write-1")
                .join("receipt.json"),
            &json!({"status": "sent"}),
            None,
        )
        .unwrap();

        let listed_free = list_profiles(&home, &data_root, |_| false).unwrap();
        assert_eq!(listed_free.len(), 1);
        assert_eq!(listed_free[0]["profile_id"], "work");
        assert_eq!(listed_free[0]["lock_status"], "free");
        assert_eq!(
            listed_free[0]["last_session_index_refresh"],
            "2026-11-25T00:00:00.000Z"
        );
        assert_eq!(listed_free[0]["chat_summary"]["conversation_count"], 2);
        assert_eq!(
            listed_free[0]["chat_summary"]["stored_conversation_count"],
            1
        );
        assert_eq!(
            listed_free[0]["content_summary"],
            json!({"user_count": 1, "video_count": 1})
        );
        let recent = listed_free[0]["recent_write_at"].as_str().unwrap();
        assert!(DateTime::parse_from_rfc3339(recent).is_ok());

        let listed_locked = list_profiles(&home, &data_root, |lock| lock == paths.lock).unwrap();
        assert_eq!(listed_locked[0]["lock_status"], "in_use");
    }

    #[test]
    fn repository_indexes_snapshots_runs_and_unknown_field_inventory() {
        let dir = TempDir::new().unwrap();
        let home = dir.path().join("appdata");
        let data_root = dir.path().join("project");
        create_profile(&home, &data_root, "work", None).unwrap();

        let convs = vec![json!({
            "conversation_id": "group:2",
            "kind": "group",
            "display_name": "Alpha/Team",
        })];
        let written = write_conversation_index(&home, &data_root, "work", &convs).unwrap();
        assert_eq!(written["count"], 1);
        assert_eq!(
            written["conversations"][0]["folder_name"],
            "group__Alpha~002FTeam__group~003A2"
        );
        let reloaded = read_conversation_index(&home, &data_root, "work")
            .unwrap()
            .unwrap();
        assert_eq!(reloaded["schema"], "doudou.conversations-index.v2");

        let snapshot = json!({
            "completed_at": "2026-11-25T15:30:45.123Z",
            "conversation_count": 1,
        });
        let snap_paths =
            write_session_index_snapshot(&home, &data_root, "work", &snapshot).unwrap();
        assert!(
            snap_paths["snapshot"]
                .as_str()
                .unwrap()
                .ends_with("20261125T153045123__session-index.json")
        );
        let latest = read_session_index_snapshot(&home, &data_root, "work")
            .unwrap()
            .unwrap();
        assert_eq!(latest["conversation_count"], 1);

        let inv = unknown_field_inventory(&[
            json!({"unknown_fields": [{"field": 77, "wire": 0}, {"field": 77, "wire": 0}]}),
            json!({"unknown_fields": [{"field": 99, "wire": 2}]}),
        ]);
        assert_eq!(inv.get("field_77_wire_0"), Some(&2));
        assert_eq!(inv.get("field_99_wire_2"), Some(&1));

        let saved_run = write_run(
            &home,
            &data_root,
            "work",
            &json!({
                "schema": "doudou.message-collect-run.v1",
                "run_id": "collect-1",
                "completed_at": "2026-11-25T15:31:00.000Z",
                "unknown_field_inventory": inv,
            }),
        )
        .unwrap();
        assert_eq!(saved_run["run_id"], "collect-1");
        let fetched = read_run(&home, &data_root, "work", "collect-1")
            .unwrap()
            .unwrap();
        assert_eq!(fetched["unknown_field_inventory"]["field_77_wire_0"], 2);
        assert_eq!(list_runs(&home, &data_root, "work").unwrap().len(), 1);
    }

    #[test]
    fn repository_message_merge_handles_conflicts_identities_and_source_priority() {
        let dir = TempDir::new().unwrap();
        let home = dir.path().join("appdata");
        let data_root = dir.path().join("project");
        create_profile(&home, &data_root, "work", None).unwrap();

        let conversation = json!({
            "conversation_id": "group:2",
            "kind": "group",
            "display_name": "group",
        });

        // 1. Authoritative fields overwrite, non-authoritative mismatch logs deduplicated conflict
        let first = json!({
            "schema": "doudou.message.v2",
            "message_id": "1",
            "create_time_ms": 1,
            "text": Value::Null,
            "type": "text",
            "message_type": 1,
            "images": [{"status": "failed"}],
        });
        let second = json!({
            "schema": "doudou.message.v2",
            "message_id": "1",
            "create_time_ms": 1,
            "text": "hello",
            "type": "image",
            "message_type": 2,
            "images": [{"status": "downloaded"}],
        });
        merge_messages(&home, &data_root, "work", &conversation, &[first]).unwrap();
        let merged = merge_messages(
            &home,
            &data_root,
            "work",
            &conversation,
            std::slice::from_ref(&second),
        )
        .unwrap();
        assert_eq!(merged["total"], 1);
        assert_eq!(merged["duplicates"], 1);
        assert_eq!(merged["conflicts"], 1);
        assert_eq!(merged["conflicts_total"], 1);

        let stored = read_messages(&home, &data_root, "work", &conversation).unwrap();
        assert_eq!(stored[0]["text"], "hello");
        assert_eq!(stored[0]["type"], "image");
        assert_eq!(stored[0]["message_type"], 1);
        assert_eq!(stored[0]["images"], json!([{"status": "downloaded"}]));

        let repeated = merge_messages(&home, &data_root, "work", &conversation, &[second]).unwrap();
        assert_eq!(repeated["conflicts"], 0);
        assert_eq!(repeated["conflicts_total"], 1);
        assert_eq!(
            read_conflicts(&home, &data_root, "work", &conversation)
                .unwrap()
                .len(),
            1
        );

        // 2. Identity objects and identity lists enrich cleanly without conflicts
        let id_conv = json!({"conversation_id": "group:3", "kind": "group", "display_name": "ids"});
        let base_msg = json!({
            "schema": "doudou.message.v2",
            "message_id": "10",
            "create_time_ms": 10,
            "sender": {"uid": "7", "sec_uid": "SEC", "unique_id": Value::Null, "name": Value::Null},
            "mentioned_users": [{"uid": "8", "unique_id": Value::Null}],
            "system_event": {
                "kind": "member_joined",
                "source_users": [{"uid": "7", "nickname": Value::Null}],
                "joined_users": [{"uid": "8", "nickname": Value::Null}],
            }
        });
        let enriched_msg = json!({
            "schema": "doudou.message.v2",
            "message_id": "10",
            "create_time_ms": 10,
            "sender": {"uid": "7", "sec_uid": "SEC", "unique_id": "douyin-7", "nickname": "Sender", "name": "露露"},
            "mentioned_users": [{"uid": "8", "unique_id": "douyin-8", "nickname": "Mentioned"}],
            "system_event": {
                "kind": "member_joined",
                "source_users": [{"uid": "7", "nickname": "Sender"}],
                "joined_users": [{"uid": "8", "nickname": "Mentioned"}],
            }
        });
        merge_messages(&home, &data_root, "work", &id_conv, &[base_msg]).unwrap();
        let res = merge_messages(&home, &data_root, "work", &id_conv, &[enriched_msg]).unwrap();
        assert_eq!(res["conflicts"], 0);
        let stored_id = read_messages(&home, &data_root, "work", &id_conv).unwrap();
        assert_eq!(stored_id[0]["sender"]["unique_id"], "douyin-7");
        assert_eq!(stored_id[0]["sender"]["name"], "露露");
        assert_eq!(stored_id[0]["mentioned_users"][0]["unique_id"], "douyin-8");
        assert_eq!(
            stored_id[0]["system_event"]["joined_users"][0]["nickname"],
            "Mentioned"
        );

        // 3. Stranger protocol upgrades summary and never downgrades back
        let req_conv =
            json!({"conversation_id": "request:2", "kind": "direct", "display_name": "req"});
        let summary = json!({
            "schema": "doudou.message.v2",
            "message_id": "1",
            "create_time_ms": 1,
            "text": "summary text",
            "type": "text",
            "images": [{"source": "summary"}],
            "collection_source": "stranger-conversation-summary",
        });
        let protocol = json!({
            "schema": "doudou.message.v2",
            "message_id": "1",
            "create_time_ms": 1,
            "text": "protocol text",
            "type": "text",
            "images": [{"source": "protocol"}],
            "collection_source": "stranger-protocol",
        });
        merge_messages(
            &home,
            &data_root,
            "work",
            &req_conv,
            std::slice::from_ref(&summary),
        )
        .unwrap();
        merge_messages(&home, &data_root, "work", &req_conv, &[protocol]).unwrap();
        merge_messages(&home, &data_root, "work", &req_conv, &[summary]).unwrap();
        let final_msg = read_messages(&home, &data_root, "work", &req_conv).unwrap();
        assert_eq!(final_msg[0]["text"], "protocol text");
        assert_eq!(final_msg[0]["images"], json!([{"source": "protocol"}]));
        assert_eq!(final_msg[0]["collection_source"], "stranger-protocol");
    }

    #[test]
    fn content_merges_comments_and_persists_checkpoints_videos_and_identities() {
        let dir = TempDir::new().unwrap();
        let home = dir.path().join("appdata");
        let data_root = dir.path().join("project");
        create_profile(&home, &data_root, "work", None).unwrap();

        // Recursive merge preserving 0 and false while keeping historical text over "" / null
        let merged = merge_video_comments(
            &[json!({
                "cid": "c1",
                "text": "historical",
                "digg_count": 9,
                "is_author_digged": true,
                "user": {"uid": "7", "nickname": "Historical Name", "sec_uid": "sec-7"},
            })],
            &[json!({
                "cid": "c1",
                "text": "",
                "digg_count": 0,
                "is_author_digged": false,
                "user": {"nickname": Value::Null, "unique_id": "new-id"},
            })],
        );
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0]["text"], "historical");
        assert_eq!(merged[0]["digg_count"], 0);
        assert_eq!(merged[0]["is_author_digged"], false);
        assert_eq!(
            merged[0]["user"],
            json!({
                "uid": "7",
                "nickname": "Historical Name",
                "sec_uid": "sec-7",
                "unique_id": "new-id",
            })
        );

        // Duplicate cids within a single incoming batch merge sequentially
        let batch_merged = merge_video_comments(
            &[json!({"cid": "old-only"})],
            &[
                json!({"cid": "new", "text": "first", "user": {"uid": "8"}}),
                json!({"cid": "new", "text": Value::Null, "user": {"nickname": "Name"}}),
                json!({"cid": "new", "text": "latest", "user": {"uid": ""}}),
            ],
        );
        assert_eq!(batch_merged.len(), 2);
        assert_eq!(batch_merged[1]["text"], "latest");
        assert_eq!(
            batch_merged[1]["user"],
            json!({"uid": "8", "nickname": "Name"})
        );

        // Comment checkpoint deduplicates partial records
        write_comment_checkpoint(
            &home,
            &data_root,
            "work",
            "video-1",
            &json!({"status": "running", "root_cursor": "20", "completed_root_pages": 1}),
            &[
                json!({"cid": "c1", "text": "old"}),
                json!({"cid": "c1", "text": "new"}),
                json!({"cid": "c2", "text": "reply"}),
            ],
        )
        .unwrap();
        let (cp, partial) = read_comment_checkpoint(&home, &data_root, "work", "video-1").unwrap();
        assert_eq!(cp.unwrap()["root_cursor"], "20");
        assert_eq!(partial.len(), 2);
        assert_eq!(partial[0]["text"], "new");

        // Identity + video index + video detail round-trip
        let identity = json!({"sec_uid": "MS4w_user", "uid": "7", "unique_id": "alice"});
        write_identity(
            &home,
            &data_root,
            "work",
            &identity,
            std::slice::from_ref(&identity),
        )
        .unwrap();
        assert_eq!(
            read_identity(&home, &data_root, "work", &identity)
                .unwrap()
                .unwrap()["unique_id"],
            "alice"
        );
        write_video_index(
            &home,
            &data_root,
            "work",
            &identity,
            &[json!({"aweme_id": "video-1"})],
            &[json!({"cursor": 0})],
        )
        .unwrap();
        assert_eq!(
            read_video_index(&home, &data_root, "work", &identity)
                .unwrap()
                .len(),
            1
        );
        let video_res = write_video(
            &home,
            &data_root,
            "work",
            &json!({"aweme_id": "video-1", "desc": "demo"}),
            Some(b"cover-bytes"),
            &partial,
            &[json!({"cursor": 0})],
        )
        .unwrap();
        assert_eq!(video_res["cover_downloaded"], true);
        assert_eq!(
            read_video_detail(&home, &data_root, "work", "video-1")
                .unwrap()
                .unwrap()["desc"],
            "demo"
        );
        assert_eq!(
            read_video_comments(&home, &data_root, "work", "video-1")
                .unwrap()
                .len(),
            2
        );
    }

    #[test]
    fn write_plan_enforces_strict_single_use_and_prevents_crash_retry_duplicate_send() {
        let dir = TempDir::new().unwrap();
        let home = dir.path().join("appdata");
        let data_root = dir.path().join("project");
        create_profile(&home, &data_root, "work", None).unwrap();

        let payload = json!({
            "conversation": {"conversation_id": "1"},
            "text": "hello",
        });
        let plan = create_plan(
            &home,
            &data_root,
            "work",
            "chat.send",
            payload.clone(),
            DEFAULT_PLAN_TTL_SECONDS,
        )
        .unwrap();
        assert_eq!(plan.status, PlanStatus::ConfirmationRequired);

        // Cannot complete a plan before confirm_plan transitions it to Executing
        let premature = complete_plan(
            &home,
            &data_root,
            "work",
            &plan.plan_id,
            TerminalPlanStatus::Sent,
            None,
        )
        .unwrap_err();
        assert_eq!(premature.code, "INVALID_PLAN_TRANSITION");

        // First confirmation atomically transitions ConfirmationRequired -> Executing
        let executing = confirm_plan(
            &home,
            &data_root,
            "work",
            &plan.plan_id,
            "chat.send",
            &payload,
        )
        .unwrap();
        assert_eq!(executing.status, PlanStatus::Executing);
        assert!(executing.confirmed_at.is_some());

        // Simulate broker crash right after external send and BEFORE complete_plan runs:
        // On-disk status remains Executing. A client retry with the exact same plan_id and payload
        // MUST be rejected with PLAN_ALREADY_USED so the write can never execute twice!
        let crash_retry = confirm_plan(
            &home,
            &data_root,
            "work",
            &plan.plan_id,
            "chat.send",
            &payload,
        )
        .unwrap_err();
        assert_eq!(crash_retry.code, "PLAN_ALREADY_USED");
        assert_eq!(
            crash_retry.details.as_ref().unwrap()["status"],
            json!("executing")
        );

        // Argument drift is still detected
        let drifted = confirm_plan(
            &home,
            &data_root,
            "work",
            &plan.plan_id,
            "chat.send",
            &json!({"conversation": {"conversation_id": "1"}, "text": "tampered"}),
        )
        .unwrap_err();
        assert_eq!(drifted.code, "PLAN_DRIFTED");

        // Complete into every terminal state and prove re-confirmation is always rejected
        for terminal in [
            TerminalPlanStatus::Sent,
            TerminalPlanStatus::Failed,
            TerminalPlanStatus::Unknown,
        ] {
            let p = create_plan(
                &home,
                &data_root,
                "work",
                "chat.send",
                payload.clone(),
                DEFAULT_PLAN_TTL_SECONDS,
            )
            .unwrap();
            confirm_plan(&home, &data_root, "work", &p.plan_id, "chat.send", &payload).unwrap();
            let done = complete_plan(
                &home,
                &data_root,
                "work",
                &p.plan_id,
                terminal,
                Some("receipt-1"),
            )
            .unwrap();
            assert_eq!(done.status, PlanStatus::from(terminal));
            assert!(done.status.is_terminal());
            assert_eq!(done.receipt.as_deref(), Some("receipt-1"));

            let replay = confirm_plan(&home, &data_root, "work", &p.plan_id, "chat.send", &payload)
                .unwrap_err();
            assert_eq!(replay.code, "PLAN_ALREADY_USED");

            let recomplete = complete_plan(
                &home,
                &data_root,
                "work",
                &p.plan_id,
                TerminalPlanStatus::Failed,
                None,
            )
            .unwrap_err();
            assert_eq!(recomplete.code, "INVALID_PLAN_TRANSITION");
        }

        // Expired plan rejection & canonical key-order-independent payload hashing
        let expired =
            create_plan(&home, &data_root, "work", "chat.send", payload.clone(), 0).unwrap();
        let exp_err = confirm_plan(
            &home,
            &data_root,
            "work",
            &expired.plan_id,
            "chat.send",
            &payload,
        )
        .unwrap_err();
        assert_eq!(exp_err.code, "PLAN_EXPIRED");

        assert_eq!(
            payload_hash("chat.send", &json!({"b": 1, "a": {"y": 2, "x": 3}})),
            payload_hash("chat.send", &json!({"a": {"x": 3, "y": 2}, "b": 1}))
        );

        // Operation mismatch check on load_plan
        let op_err =
            load_plan(&home, &data_root, "work", &plan.plan_id, Some("video.like")).unwrap_err();
        assert_eq!(op_err.code, "PLAN_OPERATION_MISMATCH");

        // Legacy on-disk `"confirmed"` plan fails closed with PLAN_ALREADY_USED
        let legacy_plan_id = "plan-legacy-confirmed";
        let paths = doudou_core::profile_paths(&home, &data_root, "work").unwrap();
        atomic_write_json(
            &paths.plans.join(format!("{legacy_plan_id}.json")),
            &json!({
                "schema": WRITE_PLAN_SCHEMA,
                "plan_id": legacy_plan_id,
                "profile_id": "work",
                "operation": "chat.send",
                "payload": payload,
                "payload_sha256": payload_hash("chat.send", &payload),
                "created_at": "2026-11-25T00:00:00.000Z",
                "expires_at": "2099-11-25T00:00:00.000Z",
                "status": "confirmed",
            }),
            None,
        )
        .unwrap();
        let legacy_err = confirm_plan(
            &home,
            &data_root,
            "work",
            legacy_plan_id,
            "chat.send",
            &payload,
        )
        .unwrap_err();
        assert_eq!(legacy_err.code, "PLAN_ALREADY_USED");
    }

    #[test]
    fn receipts_write_under_sends_and_interactions_domains() {
        let dir = TempDir::new().unwrap();
        let home = dir.path().join("appdata");
        let data_root = dir.path().join("project");
        create_profile(&home, &data_root, "work", None).unwrap();

        let send_paths = write_receipt(
            &home,
            &data_root,
            "work",
            "chat",
            "chat.send",
            &json!({"status": "sent", "verification": {"passed": true}}),
            ReceiptKind::Send,
        )
        .unwrap();
        let send_receipt_rel = send_paths["receipt"].as_str().unwrap();
        assert!(send_receipt_rel.contains("sends"));
        let send_receipt = read_receipt(&data_root.join(send_receipt_rel))
            .unwrap()
            .unwrap();
        assert_eq!(send_receipt["schema"], "doudou.write-receipt.v1");
        assert_eq!(send_receipt["status"], "sent");

        let ver_rel = send_paths["verification"].as_str().unwrap();
        let ver = read_receipt(&data_root.join(ver_rel)).unwrap().unwrap();
        assert_eq!(ver["passed"], true);

        let inter_paths = write_receipt(
            &home,
            &data_root,
            "work",
            "video",
            "video.comment",
            &json!({"status": "sent"}),
            ReceiptKind::Interaction,
        )
        .unwrap();
        let inter_rel = inter_paths["receipt"].as_str().unwrap();
        assert!(inter_rel.contains("interactions"));
        assert!(inter_rel.contains("video"));
        let inter_receipt = read_receipt(&data_root.join(inter_rel)).unwrap().unwrap();
        assert_eq!(inter_receipt["domain"], "video");
        assert_eq!(inter_receipt["operation"], "video.comment");
    }
}
