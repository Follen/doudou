//! Core primitives, error model, OS `AppData` path resolution, and identity/resolver domain logic.

pub mod error;
pub mod identity;
pub mod io;
pub mod paths;
pub mod records;
pub mod resolver;

pub use error::{DoudouError, Result};
pub use identity::{
    IDENTITY_FIELDS, IdentityCache, enrich_known_identities, enrich_message_identities,
    identity_matches, known_identity_sec_uids, merge_identity, normalize_identity,
};
pub use io::{
    absolutize_path, atomic_write_bytes, atomic_write_json, atomic_write_jsonl, atomic_write_text,
    emit, expand_tilde, is_dir_writable, normalize_lookup, random_id, read_json, read_jsonl,
    resolve_data_root, resolve_data_root_with_env, safe_segment, sha256_bytes, sha256_str, utc_now,
};
pub use paths::{
    CURRENT_IDENTITY_SCHEMA, CURRENT_PROFILE_RUNTIME, CURRENT_PROFILE_SCHEMA, ProfilePaths,
    assert_supported_identity_records, assert_supported_profile_metadata, broker_registry_root,
    doudou_home, initialize_roots, is_current_profile_metadata, profile_paths, profiles_root,
    stored_profile_paths,
};
pub use records::{Conversation, ConversationKind, Identity, conversation_kind};
pub use resolver::{
    default_local_message_window, default_message_window, infer_self_uid, parse_selection,
    resolve_conversation, resolve_message,
};

#[cfg(test)]
mod tests {
    use chrono::DateTime;
    use serde_json::json;
    use tempfile::TempDir;

    use super::*;

    #[test]
    fn safe_segment_handles_special_chars_dots_and_truncation() {
        assert_eq!(safe_segment("work", 96).unwrap(), "work");
        assert_eq!(safe_segment("a/b:c", 96).unwrap(), "a~002Fb~003Ac");
        assert!(safe_segment("..", 96).unwrap().starts_with('~'));
        assert!(safe_segment("   ", 96).is_err());

        let long = "a".repeat(120);
        let seg = safe_segment(&long, 96).unwrap();
        assert_eq!(seg.chars().count(), 96);
        assert!(seg.contains('~'));
    }

    #[test]
    fn read_jsonl_rejects_incomplete_trailing_line() {
        let dir = TempDir::new().unwrap();
        let file = dir.path().join("messages.jsonl");
        std::fs::write(&file, "{\"id\":1}").unwrap();
        let err = read_jsonl(&file, false).unwrap_err();
        assert_eq!(err.code, "INCOMPLETE_JSONL_LINE");

        atomic_write_jsonl(&file, &[json!({"id": 1}), json!({"id": 2})]).unwrap();
        let rows = read_jsonl(&file, false).unwrap();
        assert_eq!(rows.len(), 2);
    }

    #[test]
    fn doudou_home_uses_override_and_profiles_live_under_home() {
        let dir = TempDir::new().unwrap();
        let home = doudou_home(Some(dir.path()));
        let project = dir.path().join("project");
        let paths = profile_paths(&home, &project, "work").unwrap();
        assert_eq!(paths.profile, home.join("profiles").join("work"));
        assert_eq!(
            paths.browser_data,
            home.join("profiles").join("work").join("browser-data")
        );
        assert_eq!(
            paths.message,
            project.join("message").join("profiles").join("work")
        );
    }

    #[test]
    fn resolve_data_root_defaults_to_home_and_supports_overrides() {
        let dir = TempDir::new().unwrap();
        let home = absolutize_path(&dir.path().join("appdata"));
        let env_root = dir.path().join("env-data");
        let explicit_root = dir.path().join("cli-data");

        assert_eq!(resolve_data_root_with_env(None, None, &home), home);
        assert_eq!(resolve_data_root_with_env(None, Some("   "), &home), home);
        assert_eq!(
            resolve_data_root_with_env(None, Some(&env_root.display().to_string()), &home),
            absolutize_path(&env_root)
        );
        assert_eq!(
            resolve_data_root_with_env(
                Some(&explicit_root),
                Some(&env_root.display().to_string()),
                &home
            ),
            absolutize_path(&explicit_root)
        );
    }

    #[test]
    fn resolve_conversation_handles_exact_id_substring_and_ambiguity() {
        let convs = vec![
            json!({"conversation_id": "100", "display_name": "Alpha Team"}),
            json!({"conversation_id": "200", "display_name": "Alpha Ops"}),
            json!({"conversation_id": "300", "display_name": "Beta"}),
        ];
        assert_eq!(
            resolve_conversation(&convs, "200").unwrap()["display_name"],
            "Alpha Ops"
        );
        assert_eq!(
            resolve_conversation(&convs, "beta").unwrap()["conversation_id"],
            "300"
        );
        assert_eq!(
            resolve_conversation(&convs, "alpha").unwrap_err().code,
            "CONVERSATION_AMBIGUOUS"
        );
        assert_eq!(
            resolve_conversation(&convs, "missing").unwrap_err().code,
            "RECIPIENT_NOT_FOUND"
        );
    }

    #[test]
    fn parse_selection_supports_ranges_and_all() {
        assert_eq!(parse_selection("all", 3).unwrap(), vec![1, 2, 3]);
        assert_eq!(parse_selection("全部", 2).unwrap(), vec![1, 2]);
        assert_eq!(parse_selection("1, 3-4", 5).unwrap(), vec![1, 3, 4]);
        assert_eq!(
            parse_selection("4-2", 5).unwrap_err().code,
            "INVALID_CONVERSATION_RANGE"
        );
        assert_eq!(
            parse_selection("6", 5).unwrap_err().code,
            "CONVERSATION_NUMBER_OUT_OF_RANGE"
        );
    }

    #[test]
    fn default_message_window_starts_at_local_midnight() {
        let clock = DateTime::parse_from_rfc3339("2026-11-25T15:30:45+08:00").unwrap();
        let (start, end) = default_message_window(clock);
        assert_eq!(start, "2026-11-25T00:00:00+08:00");
        assert_eq!(end, "2026-11-25T15:30:45+08:00");
    }

    #[test]
    fn infer_self_uid_requires_strict_winner() {
        let convs = vec![
            json!({"conversation_type": 1, "participants": {"participants": [{"uid": "u1"}, {"uid": "u2"}]}}),
            json!({"conversation_type": 1, "participants": {"participants": [{"uid": "u1"}, {"uid": "u3"}]}}),
        ];
        assert_eq!(infer_self_uid(&convs).as_deref(), Some("u1"));

        let tied = vec![
            json!({"conversation_type": 1, "participants": {"participants": [{"uid": "u1"}, {"uid": "u2"}]}}),
        ];
        assert_eq!(infer_self_uid(&tied), None);
    }

    #[test]
    fn identity_cache_merges_and_enriches_messages() {
        let dir = TempDir::new().unwrap();
        let home = dir.path().join("appdata");
        let data_root = dir.path().join("proj");
        initialize_roots(&home, &data_root).unwrap();

        let mut cache = IdentityCache::load(&home, &data_root, "work").unwrap();
        cache.merge(
            &json!({"uid": "10", "sec_uid": "MS4w_test", "unique_id": "alice.id", "nickname": "Alice"}),
            "im-user-info",
        );
        cache.save().unwrap();

        let reloaded = IdentityCache::load(&home, &data_root, "work").unwrap();
        let exact = reloaded.exact_unique_id("ALICE.ID").unwrap();
        assert_eq!(exact["sec_uid"], "MS4w_test");
        assert_eq!(exact["enrichment"]["status"], "complete");

        let msgs = vec![json!({
            "message_id": "m1",
            "sender": {"uid": "10"},
            "content": {"richTextInfos": [{"infoType": 1, "info": {"uid": "10"}}]}
        })];
        let enriched = enrich_message_identities(&msgs, &reloaded, "chat", &[]);
        assert_eq!(enriched[0]["sender"]["unique_id"], "alice.id");
        assert_eq!(enriched[0]["mentioned_users"][0]["nickname"], "Alice");
    }

    #[test]
    fn legacy_identity_records_are_rejected() {
        let dir = TempDir::new().unwrap();
        let home = dir.path().join("appdata");
        let data_root = dir.path().join("proj");
        let paths = profile_paths(&home, &data_root, "work").unwrap();
        atomic_write_json(
            &paths.message.join("sample.json"),
            &json!({"sender": {"uid": "1"}}),
            None,
        )
        .unwrap();
        let err = assert_supported_identity_records(&home, &data_root, "work").unwrap_err();
        assert_eq!(err.code, "LEGACY_UNSUPPORTED");
    }
}
