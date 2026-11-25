# Doudou Python → Rust Porting Map (`feat/rust-rewrite`)

This document maps every module from the original Python/Camoufox implementation (`doudou/scripts/doudou/`) and every verification test suite (`docs/verification/tests/`) to the 7-crate Rust workspace, and documents the architectural upgrades introduced during the rewrite.

---

## 1. Architectural Upgrades

| Area | Legacy Python Implementation | Rust Rewrite (`feat/rust-rewrite`) |
|---|---|---|
| **Browser Engine & Stealth** | Python + Playwright + Camoufox binary download (`<DOUDOU_HOME>/browsers/`) | Native system Chrome / Chromium / Edge discovery (`doudou-cdp::chrome`) + async WebSocket CDP transport (`doudou-cdp::cdp`). **Never invokes `Runtime.enable`** (enforced at runtime via `FORBIDDEN_CDP_METHODS`); evaluates scripts in an isolated world (`Page.createIsolatedWorld`) and launches without `--enable-automation`. |
| **Profile Storage & Schema** | Project-local `profile/<name>/` (`doudou.profile.v2`, `camoufox-python`, `camoufox-config.json`) | Standard OS `AppData` (`%LOCALAPPDATA%\doudou`, `~/Library/Application Support/doudou`, `~/.local/share/doudou`), overridable via `--home` or `DOUDOU_HOME`. Profiles use schema `doudou.profile.v3`, runtime `chrome-rust`, and `chrome-config.json` (`doudou.chrome-config.v1`). Project-local business records remain under `<data_root>/message/profiles/<name>/`. |
| **Write-Safety State Machine (TLA+ Crash-Retry Fix)** | `confirmation_required -> confirmed -> sent\|failed\|unknown`; a crash between external send and completion could leave `confirmed` re-executable on retry. | Strict single-use transition `ConfirmationRequired -> Executing -> Sent\|Failed\|Unknown` (`doudou-storage::plans`) + RAII `PlanExecutionGuard` (`doudou-runtime::features::chat::service`). Re-confirming an `Executing`, terminal, or legacy `confirmed` plan always fails closed with `PLAN_ALREADY_USED`. |
| **Self-Describing CLI (`cf`-style)** | `argparse` with external markdown docs (`doudou/references/cli.md`) and a multi-hundred-line `SKILL.md` | `clap` v4 derive CLI with live `Cli::command()` introspection: `doudou cli search <query> [--json]` (intent, flag, alias, and error-code ranking) and `doudou cli schema [command...]` (machine-readable JSON schema with NDJSON & write-safety contracts). Single source of truth that cannot drift from the binary. |
| **Agent Skill Footprint** | Verbose `SKILL.md` + `references/` files duplicating CLI syntax | Ultra-slim 23-line [`doudou/SKILL.md`](doudou/SKILL.md) teaching the 4 discovery commands and 5 non-negotiable safety invariants; agents query the CLI directly for exact parameters, events, and error codes. |
| **Distribution & Runtime** | Node wrapper + Python venv + Camoufox asset installer | Single native `doudou` binary compiled from the 7-crate Rust workspace with zero Python or Playwright runtime dependency. |

---

## 2. Module-by-Module Mapping (Python → 7 Rust Crates)

### Workspace Dependency Graph

```text
doudou-core
  ├── doudou-protocol  (depends on doudou-core)
  ├── doudou-storage   (depends on doudou-core)
  ├── doudou-cdp       (depends on doudou-core)
  ├── doudou-ui        (depends on doudou-core, doudou-protocol; zero CDP dependency)
  └── doudou-runtime   (depends on core, protocol, storage, cdp, ui)
        └── doudou-cli (binary `doudou`, depends on all 6 library crates)
```

### Detailed File Mapping

| Python Module (`doudou/scripts/doudou/`) | Rust Crate & Module (`crates/`) | Notes |
|---|---|---|
| `common/errors.py` | `doudou-core/src/error.rs` | `DoudouError`, `Result<T>`, `as_event()`, `exit_code` |
| `common/core.py` | `doudou-core/src/io.rs` | Atomic temp+fsync+rename I/O, JSON/JSONL strict parsing, `safe_segment`, `normalize_lookup`, `emit`, `utc_now`, `random_id`, SHA-256 |
| `runtime/home.py`, `runtime/paths.py` | `doudou-core/src/paths.rs` | OS `AppData` `doudou_home`, `ProfilePaths`, `doudou.profile.v3` / `chrome-rust`, legacy `Profile/` & unversioned identity guards |
| `models/records.py` | `doudou-core/src/records.rs` | `Conversation`, `ConversationKind`, `Identity`, `conversation_kind` |
| `models/identity.py` | `doudou-core/src/identity.rs` | `IdentityCache`, merge/enrichment for messages and conversations |
| `models/resolver.py` | `doudou-core/src/resolver.rs` | `resolve_conversation`, `resolve_message`, `parse_selection`, `infer_self_uid`, `default_message_window` |
| `protocol/protobuf.py` | `doudou-protocol/src/protobuf.rs` | Raw wire varint/length-delimited Protobuf codec (`decode_fields`, `encode_fields`, `replace_field`) |
| `protocol/im.py` | `doudou-protocol/src/im.rs` | Douyin IM command/message/participant/stranger (cmd 1001/1002) request builders & decoders, property/reaction parsing, send request/response verification |
| `protocol/contracts.py` | `doudou-protocol/src/contracts.rs` | Web API versioned field aliases & `PROTOCOL_SCHEMA_DRIFTED` drift detection |
| `protocol/responses.py` | `doudou-protocol/src/responses.rs` | HTTP 429 `RATE_LIMITED`, `KICK` -> `VERIFICATION_REQUIRED`, JSON response classification |
| `protocol/web.py` | `doudou-protocol/src/web.rs` | Identity extraction, video/detail/comment normalizers, `RENDER_DATA` parser, comment parent resolution |
| `protocol/write_responses.py` | `doudou-protocol/src/write_responses.rs` | Form request validation & page write response parser for comments/likes |
| `storage/media.py` | `doudou-protocol/src/media.rs` | AES-256-GCM decryption, format sniffing, encrypted/plaintext MD5 fail-closed checks |
| `runtime/profile.py` | `doudou-storage/src/profile.rs` | `create_profile`, `load_profile`, `update_profile`, `delete_profile`, `list_profiles`, `chrome-config.json` |
| `storage/repository.py` | `doudou-storage/src/repository.rs` | Conversation index, session snapshots, message deduplication & conflict log, stranger source priority, run records |
| `storage/content.py` | `doudou-storage/src/content.rs` | User identity/video index/video detail/comment recursive merge & pagination checkpoints |
| `writes/plans.py` | `doudou-storage/src/plans.rs` | Canonical SHA-256 payload hashing, TTL expiry, strict single-use `ConfirmationRequired -> Executing -> Terminal` state machine |
| `writes/receipts.py` | `doudou-storage/src/receipts.rs` | Immutable write receipts under `sends/` and `interactions/` |
| `runtime/camoufox_asset.py` *(replaced)* | `doudou-cdp/src/chrome.rs` | Local Chrome/Chromium/Edge discovery (env, Windows registry/Program Files, macOS `.app`, Linux `PATH`), PE version parsing, stealth launch flags |
| `runtime/browser/session.py` | `doudou-cdp/src/cdp.rs`, `doudou-cdp/src/session.rs`, `doudou-cdp/src/fake.rs` | Native async WebSocket CDP transport (without `Runtime.enable`), isolated-world eval, `ChromeSession` headless/headed-takeover switching, `FakeBrowserSession` |
| `ui/adapters.py` | `doudou-ui/src/adapters.rs` | Versioned route/selector adapters (`AUTH`, `CHAT`, `PROFILE`, `VIDEO`) |
| `ui/common.py`, `ui/components/base.py` | `doudou-ui/src/page.rs`, `doudou-ui/src/geometry.rs`, `doudou-ui/src/mock.rs` | `UiPage`/`UiSession` traits, bounding-box/occlusion/stability verification, `click_verified_target`, `MockUiPage` |
| `ui/components/auth.py` | `doudou-ui/src/components/auth.rs` | `AuthChallengeComponent` (login, quick login, SMS/captcha challenge detection) |
| `ui/components/chat.py`, `ui/chat.py` | `doudou-ui/src/components/chat.rs`, `doudou-ui/src/chat.rs` | `ConversationListComponent`, `ChatComposerComponent`, `MessageItemComponent`, `ImagePreviewComponent`, `send_message`, `like_message` |
| `ui/components/video.py`, `ui/video.py` | `doudou-ui/src/components/video.rs`, `doudou-ui/src/video.rs` | `TutorialDialogComponent`, `VideoCommentsPanelComponent`, `VideoCommentComposerComponent`, `CommentItemComponent`, `comment_write`, `like_comment` |
| `runtime/process.py`, `runtime/platform/windows.py` | `doudou-runtime/src/process.rs` | Cross-platform PID + creation-time identity, Windows UTF-8 PowerShell process query, Linux `/proc` zombie detection |
| `runtime/lock.py` | `doudou-runtime/src/lock.rs` | Atomic exclusive `.lock.json` profile lock with live PID verification and stale reclamation |
| `runtime/broker/server.py`, `client.py`, `registration.py` | `doudou-runtime/src/broker/{protocol,registry,server,client}.rs` | Loopback TCP JSON-line broker, 16 MiB bounded frame limits, constant-time auth token, single-flight lock, idle exit, `BrokerLauncher`, global `<doudou_home>/brokers/` registry |
| *(new bridge)* | `doudou-runtime/src/cdp_ui_bridge.rs` | Bridges `doudou-cdp::CdpPage` + `ChromeSession` onto `doudou-ui::{UiPage, UiSession}` |
| `reads/transport.py`, `reads/chat.py`, `reads/video.py` | `doudou-runtime/src/reads/{transport,chat,video}.rs` | Protocol JSON/Protobuf fetchers, IM conversation/message/stranger collectors, identity & video/comment paginated collectors |
| `features/chat/service.py` | `doudou-runtime/src/features/chat/{service,batch}.rs` | Chat preview/execute services, `PlanExecutionGuard`, protocol read-back verification, >=5s spaced stop-on-first-failure batch sender |
| `features/video/service.py` | `doudou-runtime/src/features/video/service.rs` | Identity/video collection, comment create/reply/like plans & read-back verification |
| `runtime/environment.py`, `features/diagnostics.py` | `doudou-runtime/src/features/diagnostics.rs` | `doctor` (with optional live headless Chrome CDP `--launch-probe`), `ui_probe` (`page-doctor`), `protocol_capture` |
| `features/broker.py` | `doudou-runtime/src/features/dispatch.rs` | `DefaultBrokerHandler` routing all 16 broker RPC commands |
| `cli.py` | `doudou-cli/src/{args,dispatch,introspect,lib,main}.rs` | 100% CLI parity + `broker serve`/`list`/`stop-all`, `profile delete`, `session login`/`close`, `cf`-style `cli search` & `cli schema` |

---

## 3. Test-by-Test Mapping (All 19 Python Suites → Rust Tests)

| # | Python Test Suite (`docs/verification/tests/`) | Py Tests | Rust Equivalent Crate & Test Module / Functions |
|---|---|---:|---|
| 1 | `test_architecture.py` | 9 | Enforced structurally by Cargo workspace DAG (`doudou-ui` has zero CDP dependency; `doudou-core` has zero browser dependency) + `doudou-cdp` `cdp_client_multiplexes_captures_events_and_never_sends_runtime_enable` + `doudou-ui/tests/write_flows.rs::adapters_match_versions_selectors_routes_and_require` + `doudou-cli/tests/cli_e2e.rs::cli_schema_emits_full_and_scoped_machine_readable_json` |
| 2 | `test_batch.py` | 2 | `doudou-runtime/tests/batch_and_chat_tests.rs`: `batch_first_failure_stops_and_marks_remaining_items_skipped`, `batch_successful_attempts_are_spaced_by_at_least_five_seconds` |
| 3 | `test_broker_frame_limits.py` | 3 | `doudou-runtime/src/broker/protocol.rs::tests::read_bounded_line_enforces_exact_limit` + `doudou-runtime/tests/broker_tests.rs`: `client_frame_limits_accept_128kib_and_reject_oversized_read_and_write`, `invalid_token_and_oversized_request_frame_are_rejected_cleanly` |
| 4 | `test_broker_registration.py` | 2 | `doudou-runtime/tests/broker_tests.rs::broker_registration_lists_queries_and_stops_brokers` + `doudou-cli/tests/cli_e2e.rs::broker_launcher_spawns_serve_subprocess_and_handles_fake_browser_rpc` |
| 5 | `test_broker.py` | 34 | `doudou-runtime/src/broker/protocol.rs::tests::command_timeouts_match_contract`, `doudou-runtime/src/lock.rs::tests::profile_lock_acquires_rejects_live_duplicate_and_reclaims_stale`, `doudou-runtime/tests/broker_tests.rs` (all 9 integration tests covering concurrency, at-most-once write non-replay, cancellation, takeover recovery, idle exit, timeouts, deterministic write errors), `doudou-cdp/src/fake.rs::tests` (5 session/takeover/retry tests) |
| 6 | `test_cli_doctor.py` | 6 | `doudou-cli/tests/cli_e2e.rs`: `doctor_does_not_launch_browser_by_default_and_launch_probe_is_explicit`, `doctor_rejects_legacy_profile_casing_before_probe`, `identity_legacy_record_fails_before_broker_start`, `identity_migrate_and_warmup_are_not_cli_commands_and_session_refresh_exists`, `legacy_at_mention_fails_before_broker_start`, `doctor_offline_returns_structured_ndjson_with_status_and_chrome_discovery` |
| 7 | `test_component_geometry.py` | 27 | `doudou-ui/tests/component_geometry.rs` (29 integration tests covering occlusion, scope tolerance, dynamic center stability, scroll-before-validate, retry bounds, chat/video composer focus & Enter send, emoji semantics) |
| 8 | `test_core.py` | 3 | `doudou-core/src/lib.rs::tests`: `safe_segment_handles_special_chars_dots_and_truncation`, `read_jsonl_rejects_incomplete_trailing_line`, `resolve_conversation_handles_exact_id_substring_and_ambiguity`, `parse_selection_supports_ranges_and_all` |
| 9 | `test_identity.py` | 11 | `doudou-core/src/lib.rs::tests`: `identity_cache_merges_and_enriches_messages`, `legacy_identity_records_are_rejected`; `doudou-storage/src/lib.rs::tests::repository_message_merge_handles_conflicts_identities_and_source_priority` |
| 10 | `test_profile.py` | 9 | `doudou-storage/src/lib.rs::tests`: `profile_crud_uses_appdata_home_chrome_config_and_v3_schema`, `profile_rejects_legacy_root_and_legacy_metadata_without_mutation`, `list_profiles_reports_summaries_lock_probe_and_skips_legacy`; `doudou-cli/tests/cli_e2e.rs::profile_create_list_status_delete_respects_home_and_data_root_separation` |
| 11 | `test_protocol_storage.py` | 24 | `doudou-protocol/src/lib.rs::tests`: `protobuf_round_trip_and_stranger_envelope_mapping`, `im_send_request_and_response_round_trip`, `aes_gcm_image_decryption_and_md5_verification_pass_and_fail_closed`; `doudou-storage/src/lib.rs::tests`: `repository_indexes_snapshots_runs_and_unknown_field_inventory`, `repository_message_merge_handles_conflicts_identities_and_source_priority`, `receipts_write_under_sends_and_interactions_domains` |
| 12 | `test_responses.py` | 6 | `doudou-protocol/src/lib.rs::tests::response_classifier_separates_rate_limit_kick_and_schema_drift`; `doudou-ui/tests/write_flows.rs::business_failure_and_kick_are_known` |
| 13 | `test_runtime_home.py` | 3 | `doudou-core/src/lib.rs::tests::doudou_home_uses_override_and_profiles_live_under_home`; `doudou-runtime/tests/process_and_home_tests.rs`: `runtime_home_controls_profiles_and_broker_registry_roots`, `doctor_reports_writable_roots_profiles_brokers_and_chrome_discovery` |
| 14 | `test_stranger_chat_integration.py` | 11 | `doudou-runtime/tests/batch_and_chat_tests.rs`: `stranger_collect_routes_1002_and_falls_back_to_summary_when_empty`, `stranger_display_name_filtering_wire_format_and_readonly_write_guard`, `stranger_refresh_failure_and_stalled_cursor_preserve_previous_index` |
| 15 | `test_stranger_protocol.py` | 3 | `doudou-protocol/src/lib.rs::tests::protobuf_round_trip_and_stranger_envelope_mapping`; `doudou-storage/src/lib.rs::tests::repository_message_merge_handles_conflicts_identities_and_source_priority` (source-priority upgrade/no-downgrade) |
| 16 | `test_video_comment_pagination.py` | 4 | `doudou-storage/src/lib.rs::tests::content_merges_comments_and_persists_checkpoints_videos_and_identities` (recursive comment merge, duplicate cid deduplication, partial checkpoint persistence) |
| 17 | `test_video_writes.py` | 8 | `doudou-ui/tests/write_flows.rs` (11 tests covering chat/video write flows, captured write validation, comment like direction, request drift -> unknown, auth challenges); `doudou-runtime/tests/batch_and_chat_tests.rs::video_interaction_plans_and_readback_verification_work` |
| 18 | `test_web_plans_cli.py` | 6 | `doudou-core/src/lib.rs::tests`: `default_message_window_starts_at_local_midnight`, `infer_self_uid_requires_strict_winner`; `doudou-storage/src/lib.rs::tests::write_plan_enforces_strict_single_use_and_prevents_crash_retry_duplicate_send`; `doudou-cli/tests/cli_e2e.rs`: `identity_migrate_and_warmup_are_not_cli_commands_and_session_refresh_exists`, `confirm_without_plan_id_fails_with_plan_id_required` |
| 19 | `test_windows_process.py` | 5 | `doudou-runtime/src/process.rs::tests` (4 unit tests) + `doudou-runtime/tests/process_and_home_tests.rs`: `windows_background_mode_is_true_headless`, `windows_process_identity_pins_utf8_handles_empty_and_parses_unicode_json` |
| **+** | **New Rust-Specific Coverage** | — | `doudou-cdp/src/chrome.rs::tests` (11 Chrome discovery/stealth/PE version/DevToolsActivePort tests), `doudou-cdp/src/cdp.rs::tests` (2 CDP multiplexing & no-`Runtime.enable` tests), `doudou-runtime/tests/batch_and_chat_tests.rs::cdp_ui_page_bridge_translates_queries_clicks_keys_and_network_capture`, `doudou-cli/tests/cli_e2e.rs` (`--version`, `--help`, `cli search`, `cli schema`, real subprocess `broker serve` E2E) |
