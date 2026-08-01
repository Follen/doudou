# Acceptance evidence

<!-- comet-native:acceptance-evidence:start -->
[
  {
    "acceptance_id": "acceptance-4542b75f9ed182f6f5324ca6558b87eaf9a265cd50f9df667530b505895661d7",
    "evidence_refs": [
      "douyin-message/src/commands.js",
      "douyin-message/src/im-client.js",
      "douyin-message/test/cli.test.js",
      "douyin-message/test/contract.test.js"
    ]
  },
  {
    "acceptance_id": "acceptance-4ea34f9338be7aa211e0c15d5a01641a501a2a5a75c123631745d828d8d1157d",
    "evidence_refs": [
      "analyze/protocol-media-mentions-verification.md",
      "douyin-message/src/im-client.js",
      "douyin-message/src/media.js",
      "douyin-message/test/media.test.js"
    ]
  },
  {
    "acceptance_id": "acceptance-614f23809894fc692f6aabdbf08bc39fad56e2dc6f590a5556aa1ef25e799c64",
    "evidence_refs": [
      "analyze/protocol-media-mentions-verification.md",
      "douyin-message/src/im-client.js",
      "douyin-message/src/protocol.js",
      "douyin-message/test/contract.test.js"
    ]
  },
  {
    "acceptance_id": "acceptance-753f35d1fba7d8b3da409c8dbb486f56a8fdafa0630a554947c609b59b951ce0",
    "evidence_refs": [
      "douyin-message/src/media.js",
      "douyin-message/src/storage.js",
      "douyin-message/test/media.test.js",
      "douyin-message/test/storage.test.js"
    ]
  },
  {
    "acceptance_id": "acceptance-a87baba837a07ef35c5c682424e74d8c20d5cc10dcdb3a543f6c26af2650f7f2",
    "evidence_refs": [
      "analyze/protocol-media-mentions-verification.md",
      "douyin-message/src/im-client.js",
      "douyin-message/test/contract.test.js"
    ]
  },
  {
    "acceptance_id": "acceptance-f839400f2e651bba84235100b5818c3eaa710732697663aabc32b4baa6cc3dc9",
    "evidence_refs": [
      "analyze/protocol-media-mentions-verification.md",
      "douyin-message/src/media.js",
      "douyin-message/src/protocol.js",
      "douyin-message/test/media.test.js"
    ]
  }
]
<!-- comet-native:acceptance-evidence:end -->

# Commands and results

- `npm test` in `douyin-message/`: exit `0`; 62 tests passed, 0 failed, 0 skipped.
- `npm pack --dry-run --json` in `douyin-message/`: exit `0`; 32 package entries; no `profile/`, `message/`, `analyze/`, `node_modules/` or browser data included.
- `comet native check im-media-and-mentions`: exit `0`; receipt `runtime/evidence/check-receipts/208419d74c11726c21033941b9373ef4482fe77ef692321adbbd1dcc96d32714.json`.
- Existing production plain-text receipt from the current implementation: mode `plain`, HTTP `200`, Protobuf status `0`, server result `0`, matching client message ID, selected conversation match and `dom_operations_zero=true`.
- Existing protocol experiment report: mention send passed with HTTP `200`, Protobuf status `0`, server result `0`, matching client message ID and zero DOM operations.
- Existing protocol experiment report: image upload/send passed through upload config, Apply, TOS upload, Commit, image build and `message_type=27` send.
- Existing protocol experiment report: the same image was pulled through command `301`, decrypted with AES-256-GCM, and passed format, length and MD5 checks.

# Skipped checks

- No additional live reference reply was sent in this verification round. The current builder's field `11` reference payload and zero-DOM call boundary are covered by contract tests; a previous real reference-reply receipt exists in project-local runtime data.
- PNG was used for the live closed-loop experiment. JPEG and WebP format handling are covered by deterministic tests rather than additional external messages.
- Failure-path media tests use local fixtures; no intentionally corrupted media was uploaded to the live service.

# Spec consistency

- Plain text and reference reply now share direct `SendMessageRequestBody` construction and page-context `/v1/message/send`; reply adds field `11` `ref_msg_info` and does not use the composer.
- Mention sends resolve protocol participants, encode UTF-16 ranges and repeated user IDs, and reject direct conversations.
- Image sends use the protocol upload chain and image reads use command `301`, authenticated decryption, integrity checks and atomic project-local persistence.
- Confirmed writes retain preview plus matching plan ID. Batch sends remain sequential with a minimum five-second interval.
- The formal message workflow contains no composer fill, send-button click, file-input operation, conversation-DOM open or React object traversal.

# Known limitations and risks

- The browser remains necessary as the authenticated `/chat` security context for Cookie and dynamic signing parameters; message business operations themselves are protocol calls.
- Live site protocol fields and upload configuration can drift. Stable errors, capture tooling and page doctor remain available for diagnosis.
- A live reference-reply rerun requires a valid current Profile login and an explicit target/text confirmation; no duplicate external message was created during this verification.

# Conclusion

Pass. Plain text and reference reply are implemented as pure protocol message operations, with the same server receipt checks as mention and image sends. Automated tests, package boundary checks, Comet hygiene and the existing live protocol experiments all passed within the stated limits.
