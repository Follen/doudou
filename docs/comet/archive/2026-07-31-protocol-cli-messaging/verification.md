# Acceptance evidence

<!-- comet-native:acceptance-evidence:start -->
[
  {
    "acceptance_id": "acceptance-0eb1da02cd53304a33a05aa2caf96d933a40e81d711ba434e241c3bec3e3387a",
    "evidence_refs": [
      "douyin-message/src/adapter.js",
      "douyin-message/test/contract.test.js"
    ]
  },
  {
    "acceptance_id": "acceptance-3b6a0a18c67826cb1ae883764f7d30e4b391cb42f60872a30b05c733e309eed3",
    "evidence_refs": [
      "douyin-message/src/commands.js",
      "douyin-message/test/cli.test.js",
      "douyin-message/test/contract.test.js"
    ]
  },
  {
    "acceptance_id": "acceptance-3e43761ab8ea1d3a1369366e9a61ef958f8db27357a45c633f7f9df66d649753",
    "evidence_refs": [],
    "skipped_reason": "未执行需要真实页面发送的有头协议 capture；交付验证明确不发送真实消息。"
  },
  {
    "acceptance_id": "acceptance-497d539d44e7c8d347ff1d00c5c1f3f316e7cda60f7a42670c27263285d9ddff",
    "evidence_refs": [
      "douyin-message/src/doctor.js",
      "douyin-message/test/cli.test.js"
    ]
  },
  {
    "acceptance_id": "acceptance-610138697913313574462ca12c6c67562b203ce6944b4e737c2accf50f897eb5",
    "evidence_refs": [
      "douyin-message/src/fingerprint.js",
      "douyin-message/src/profile.js",
      "douyin-message/test/fingerprint.test.js",
      "douyin-message/test/profile.test.js"
    ]
  },
  {
    "acceptance_id": "acceptance-b612cc84070a6c9f191cd97045249fb75ead1a3515688648ccbfa1df18af34a7",
    "evidence_refs": [],
    "skipped_reason": "未执行真实普通发送或引用回复；自动化仅验证预览、plan 完整性、Protobuf 改写和回执判定。"
  },
  {
    "acceptance_id": "acceptance-bf323c937a295f28af05503141a6b8915c33f07fa1835bc37dd12b330e86e76c",
    "evidence_refs": [
      "douyin-message/src/page-doctor.js",
      "douyin-message/test/contract.test.js"
    ]
  },
  {
    "acceptance_id": "acceptance-e8c32516204da41de7fbd53c863775cb37fb5f88deeb0d4ebf4fcc5967332e4e",
    "evidence_refs": [
      "douyin-message/npm-shrinkwrap.json",
      "douyin-message/src/doctor.js",
      "douyin-message/test/cli.test.js"
    ]
  },
  {
    "acceptance_id": "acceptance-e9320dc40df09ecbb1e8e7dadc546638c05825a2c97e4ff9d4fb5c58fc2b9942",
    "evidence_refs": [
      "douyin-message/SKILL.md",
      "douyin-message/package.json"
    ]
  },
  {
    "acceptance_id": "acceptance-f8560d7a19d9ed3f37968d633e9e35a8331092cdbcec46dc71959429449e8be1",
    "evidence_refs": [
      "douyin-message/src/commands.js",
      "douyin-message/src/storage.js",
      "douyin-message/test/storage.test.js"
    ]
  }
]
<!-- comet-native:acceptance-evidence:end -->

# Commands and results

- `cd douyin-message && npm ci && npm test`: exit `0`; 38 tests passed, 0 failed, 0 skipped; npm reported 0 vulnerabilities.
- `python .../skill-creator/scripts/quick_validate.py D:/Code/douyin-group-message/douyin-message`: exit `0`; `Skill is valid!`.
- Isolated `npm pack`: exit `0`; 25 files, 59,222 bytes, SHA-1 `082ab927185c308e0f1329fa39b17fb99da0dc4a`.
- Isolated doctor before setup: exit `0`; result `setup_required`, expected Playwright `1.62.0`, installed version `null`.
- Isolated CLI setup: exit `0`; `npm_ci=completed`, `chromium_install=completed`.
- Isolated doctor after setup: exit `0`; Node `24.13.1`, npm `11.18.0`, Playwright `1.62.0`, Chromium `151.0.7922.34`, launch `true`, issues empty.
- Isolated layout audit: user root contained exactly `Profile` and `message`; unpacked Skill contained no `Profile`, `message` or `analyze`.
- Full literal summaries and normalized temporary paths are recorded in `verification-record.md`.

# Skipped checks

- Headed login with a newly created real account session was not repeated; Profile and fingerprint lifecycle are covered by automated tests.
- Live group/direct conversation collection was not repeated; selection, Protobuf parsing, incremental merge and per-conversation failure behavior are covered by automated tests.
- Live protocol capture was skipped because it requires the user to perform a real page send.
- Real plain send and reference reply were skipped; no external message was sent during delivery verification.

# Spec consistency

The implementation matches the confirmed target layout and command surface. The audit found and fixed a direct-message gap: Protobuf message extraction previously accepted only `conversation_type=2`; it now accepts types `1` and `2` and rejects a response whose type does not match the selected conversation. The final Skill uses bounded DOM fallback and contains no React/Fiber traversal.

# Known limitations and risks

Build uses the user-confirmed partial scope `5caf0e1e98c1d45bc1024f67d6e898759d8e2379bc298fa52d79224ba0b654e3`. The final `douyin-message/` artifact and `.gitignore` are hashed, but 30 removed root-level legacy files cannot be attributed because Runtime requires artifacts to exist. Live page behavior can drift after this verification; capture and page doctor are the supported diagnostic paths.

# Conclusion

Pass for the deliverable Skill, CLI, isolated setup, browser doctor, project-local data boundary, group/direct parsing, persistence, storage and non-live send contracts. Live capture and real sending remain explicitly unverified and were not used to claim success.
