# Acceptance evidence

<!-- comet-native:acceptance-evidence:start -->
[
  {
    "acceptance_id": "acceptance-45470a181f42131b871a769f67eba218a6ead10be2ccfc4e66b85e6a5da0b42c",
    "evidence_refs": [
      "douyin-message/src/warmup.js",
      "douyin-message/test/cli.test.js",
      "douyin-message/test/warmup.test.js"
    ]
  },
  {
    "acceptance_id": "acceptance-6fce1f875ea7a7caa184e74afe2ec29581a40c6c268895863ed0e9b1a6e5358c",
    "evidence_refs": [
      "douyin-message/src/resolver.js",
      "douyin-message/test/cli.test.js",
      "douyin-message/test/resolver.test.js"
    ]
  },
  {
    "acceptance_id": "acceptance-73cfeb632641668cb134c93e385312141244155d4bb536ba55168c1a69ef9506",
    "evidence_refs": [
      "douyin-message/src/commands.js",
      "douyin-message/src/resolver.js",
      "douyin-message/test/resolver.test.js"
    ]
  },
  {
    "acceptance_id": "acceptance-8e34f71649e95e55a74d4175da8cce7533412909e3ca6cae5698cb62854e8db1",
    "evidence_refs": [
      "douyin-message/src/protocol.js",
      "douyin-message/test/cli.test.js",
      "douyin-message/test/contract.test.js"
    ]
  },
  {
    "acceptance_id": "acceptance-dc030635ecadbcef1578d437dc868f429ef4d66d324765e79a0b75b8e68864af",
    "evidence_refs": [
      "douyin-message/src/commands.js",
      "douyin-message/test/cli.test.js",
      "douyin-message/test/resolver.test.js"
    ]
  }
]
<!-- comet-native:acceptance-evidence:end -->

# Commands and results

- `npm test` in `.agents/skills/douyin-message`: exit 0; 49 passed, 0 failed.
- `npm pack --dry-run --json` in `douyin-message`: exit 0; 29 files, no Profile, message, analyze, or node_modules content.
- Standalone headless warmup with the existing project Profile: exit 0; source `live`; 22 conversations resolved (18 direct, 4 group), 0 unresolved.
- Conversation resolution by a normalized name substring: exit 0; one existing direct conversation selected by `name_contains`.
- Message collection by conversation name with no explicit range: exit 0; local-midnight default applied; one conversation completed, 4 new messages, 0 failed conversations.
- Protocol send preview by recipient name: exit 0; returned `confirmation_required` with a deterministic plan ID; no write executed.
- Protocol reference-reply preview by message keyword: exit 0; one message selected by `text_contains`; returned `confirmation_required`; no write executed.
- Existing confirmed protocol-send verification record: HTTP and Protobuf checks passed; server accepted the message and echoed the SDK client message ID.
- Structured warmup snapshot inspection: zero forbidden credential keys and zero URLs containing query strings, fragments, or credentials.
- `comet native check prewarm-douyin-message-workflows --json`: exit 0; receipt `runtime/evidence/check-receipts/b2249051a9bcd511c356ab7ec65153ad2057aad459fdce2e818ef21069f87c50.json`; 24 text files scanned, 0 issues, result fresh.

# Skipped checks

- No new confirmed message was sent during this verification because the existing server-verified send receipt covers the unchanged write path and a second user-visible message would be duplicate side effect.
- The two-hour stale-cache behavior was exercised deterministically in automated tests rather than waiting two wall-clock hours.

# Spec consistency

- Collection, legacy reply, batch reply, protocol send, and protocol reply use the common warmup implementation before business work.
- Warmup captures environment, login, conversation/user mapping, Protobuf responses, and bounded DOM/network contract data without React/Fiber traversal.
- Existing-conversation and message resolvers implement normalized substring matching, stable ambiguity candidates, `RECIPIENT_NOT_FOUND`, and the local-day default range.
- Stale successful snapshots are accepted only through two hours for reads and previews; confirmed writes request live warmup with stale fallback disabled.
- Source and installed Skill contained 30 matching files by SHA-256 before packaging; the source delivery directory excludes runtime data and dependency cache.

# Known limitations and risks

- Recipient lookup intentionally covers only conversations present in warmup; it does not search globally or create a new direct conversation.
- Site contract changes can still require headed protocol capture or login recovery; the CLI reports stable repair commands and keeps write confirmation mandatory.
- Actual sends and replies require a successful fresh live warmup even when a usable stale snapshot exists.

# Conclusion

Pass. All five acceptance examples are covered by code, automated tests, sanitized packaging checks, real headless Profile execution, confirmation previews, and an existing verified protocol-send receipt.
