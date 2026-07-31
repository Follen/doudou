# Acceptance evidence

<!-- comet-native:acceptance-evidence:start -->
[
  {
    "acceptance_id": "acceptance-430aa73efb13048883326068b8855c798c366ac8d1c555ae2cf6b4b330179804",
    "evidence_refs": [
      "douyin-message/src/protocol.js"
    ]
  },
  {
    "acceptance_id": "acceptance-4857f013fd1fb130cb771a6b011177b124a9f2b3d3d39622bbf627bffdee615e",
    "evidence_refs": [
      "douyin-message/src/adapter.js",
      "douyin-message/test/contract.test.js"
    ]
  },
  {
    "acceptance_id": "acceptance-73e8de7611a5fbc6a6c80622b268d69b2c562227931f46b2681fc055b2d0455f",
    "evidence_refs": [
      "douyin-message/src/adapter.js",
      "douyin-message/src/warmup.js"
    ]
  },
  {
    "acceptance_id": "acceptance-ad4edd39724dc8e81cb4bdfcfefb5b5015dae51f2b1933d73723cfaa73dcb0d5",
    "evidence_refs": [
      "douyin-message/src/protocol.js"
    ]
  },
  {
    "acceptance_id": "acceptance-c1e4900e4baedd9374425e839a640a1996306a8e99f5fb1e7080edef5ab2fbda",
    "evidence_refs": [
      "douyin-message/src/protocol.js",
      "douyin-message/test/contract.test.js"
    ]
  },
  {
    "acceptance_id": "acceptance-d25d272020b43dbd9f401a94889d9a7dec34ff8f0e057a9d9aedaa4a55d25279",
    "evidence_refs": [
      "douyin-message/src/protocol.js"
    ]
  }
]
<!-- comet-native:acceptance-evidence:end -->

# Commands and results

- `NODE_PATH=<installed skill node_modules> npm test --prefix douyin-message`: passed, 50 tests.
- `node --check` for adapter, protocol, commands, warmup and page-doctor: passed.
- Source-to-installed-Skill SHA-256 comparison: passed with no mismatches.
- `rg` for `openHome`, `openMessages` and `HOME_URL` across the delivery and installed Skill: no matches.
- `comet native check chat-protocol-first`: passed; 7 scoped files scanned with 0 findings.
- Installed CLI `warmup --profile Follen-douyin`: passed live through `/chat`; the snapshot reported 22 active conversations (18 direct and 4 group), and its contract recorded requested and final paths as `/chat`.
- Installed CLI `protocol send` preview for an existing direct conversation: returned `confirmation_required` after live `/chat` warmup and did not write a message.
- The local `/chat` protocol research run sent one explicit test message with an aborted SDK bootstrap followed by page-context `fetch`; the server returned HTTP 200 and successful Protobuf status. Its sanitized report is local-only under `analyze/diagnostics/`.

# Skipped checks

- No additional confirmed group send or reference-reply write was run during this verification to avoid creating extra user-visible test messages. Their common request rewriting path is covered by the protocol unit test; a future user-confirmed send exercises the same receipt checks.
- A stable public page signer callable was not found. The implemented and tested contract retains the single aborted SDK bootstrap request before page-context direct send.

# Spec consistency

The implementation replaces all delivery-source homepage and message-panel navigation with `openChat()`. The send flow creates a current SDK template, aborts that template, rewrites the Protobuf body, and performs the final request in page context. Receipt fields use endpoint shape and hashes rather than raw signing values or identifiers. The installed Skill is byte-for-byte synchronized with the delivery directory excluding its dependency installation.

# Known limitations and risks

The `/chat` application may itself restore a previously selected conversation and issue service-side reads; the CLI does not call mark-read or other read-side write endpoints. A future page contract change can prevent template capture; that path fails before the final request rather than falling back to the homepage flow.

# Conclusion

Pass. The `/chat` protocol-first entry, SDK-bootstrap interception, page-context send, receipt redaction and installed-Skill synchronization are implemented and verified within the stated limits.
