---
name: doudou
description: Manage a project-local Douyin browser Profile and use a deterministic CLI from https://www.douyin.com/chat to discover group or direct conversations, collect messages under message/, send plain messages or reference replies through page-context protocol requests, and diagnose runtime contract changes. Use when the user asks to set up or log in to Douyin, inspect chats, pull message history, send or reply to a message, inspect Protobuf fields, or diagnose Node, npm, Playwright, Chromium, browser fingerprint, DOM, or IM-contract changes.
---

# Doudou

Run commands from the user's active project root with `node <skill-directory>/bin/doudou.js`; do not change the working directory to the Skill folder. Treat stdout as NDJSON. The current project directory is the default data root. Use `--data-root PATH` only when the user explicitly selects another project root.

## Start Every Task

1. Run `node <skill-directory>/bin/doudou.js doctor` from the project root.
2. If the result is `setup_required`, run `node <skill-directory>/bin/doudou.js setup`, rerun doctor, then resume the original command.
3. Run `node <skill-directory>/bin/doudou.js profile list`.
4. If no Profile exists, create one and open headed login:

```text
node <skill-directory>/bin/doudou.js profile create --profile <name>
node <skill-directory>/bin/doudou.js profile login --profile <name>
```

5. Reuse the selected Profile and its fingerprint. Login, calibration and manual verification are headed; discovery, collection and protocol sending are headless.
6. Before collection, send, or reply, let the business command run its built-in warmup. Use the standalone command only when the user asks to inspect readiness:

```text
node <skill-directory>/bin/doudou.js warmup --profile <name> --allow-stale
```

Warmup checks the environment, Profile, fingerprint and login, then refreshes conversation, user and bounded page/network contract metadata. It never bulk-downloads every conversation's messages.

Read [layout.md](references/layout.md) before moving data, packaging the Skill, interpreting evidence paths, or changing filenames. Read [protobuf-message-fields.md](references/protobuf-message-fields.md) when decoding messages or changing protocol builders.

## Discover Conversations

Refresh all group and direct conversations:

```text
node <skill-directory>/bin/doudou.js conversations refresh --profile <name> --kind all
```

Resolve an existing conversation by ID or a case-insensitive, whitespace-normalized name substring:

```text
node <skill-directory>/bin/doudou.js conversations resolve --profile <name> --target <id-or-name>
```

Multiple substring matches are returned as numbered candidates, including when one candidate is an exact name. `RECIPIENT_NOT_FOUND` means no existing conversation matches. Do not use global user search or create a new direct conversation.

Report `count`, then number `result.conversations` in returned order. Resolve user selections only against the same `refreshed_at`:

```text
node <skill-directory>/bin/doudou.js conversations select --profile <name> --refreshed-at <time> --selection <numbers|ranges|全部>
```

Do not classify chats from title text. Trust Protobuf `conversation_type` first (`1` direct, `2` group), then bounded explicit DOM attributes/avatar structure. Do not traverse React Fiber or arbitrary object graphs from DOM nodes.

## Collect Messages

Use a name or IDs from the latest warmup index. Omitting both boundaries collects from local midnight through command time:

```text
node <skill-directory>/bin/doudou.js messages collect --profile <name> --conversation <name> [--from <time> --to <time>]
node <skill-directory>/bin/doudou.js messages collect --profile <name> --conversations <id,id> [--from <time> --to <time>]
```

Collection captures the automatic `/chat` initialization envelope, then pages command `301` through `/v1/message/get_by_conversation`. It does not open, click or scroll a conversation. Image messages are downloaded automatically, decrypted with AES-256-GCM, verified and stored under the conversation's `media/` directory. Render `conversation_progress` in order. A failure in one conversation does not stop the remaining read-only collections. Report new, duplicate, conflict, skipped and failed counts plus `run_file` and output directories.

## Send Through Protocol

Plain send:

```text
node <skill-directory>/bin/doudou.js protocol send --profile <name> --recipient <existing-name> --text <text>
```

Mention one or more group members. Repeat `--at` or use comma-separated names:

```text
node <skill-directory>/bin/doudou.js protocol send --profile <name> --recipient <group-name> --text <text> --at <member> --at <member>
```

Send one PNG, JPEG or WebP image:

```text
node <skill-directory>/bin/doudou.js protocol send --profile <name> --recipient <existing-name> --image <file>
```

Send the same text to multiple existing conversations:

```text
node <skill-directory>/bin/doudou.js protocol send-batch --profile <name> --conversation-ids <id,id> --text <text>
```

Batch send returns one complete preview and requires one confirmation. After confirmation it performs one live warmup, holds one Profile lock, reuses one headless browser session, and sends strictly in plan order. It waits exactly 5 seconds between successful sends. Each send directly builds its own Protobuf request and verifies its server receipt. Any failed or unverifiable send stops the remaining tasks; report sent, failed and skipped entries from the returned results. Do not implement batch delivery by looping over `protocol send` commands.

Reference reply to one collected message:

```text
node <skill-directory>/bin/doudou.js protocol reply --profile <name> --recipient <existing-name> --message-text <keyword> --text <text>
```

Reply keyword matching ignores case and collapses consecutive spaces, tabs and newlines. One match proceeds to preview; multiple matches return numbered message candidates. `--conversation-id` and `--message-id` remain available for exact selection.

Send and reply commands first return `confirmation_required`. Show the exact preview and obtain explicit confirmation. Then rerun with the returned `--confirm --plan-id <id>`. Preview generation prefers a successful warmup snapshot no older than two hours; confirmed writes perform one live warmup for the command. The CLI captures only the automatic init transport envelope, obtains the conversation ticket from `ConversationInfoV2`, directly builds `SendMessageRequestBody`, and submits it with page-context `fetch`. Plain text, reference replies, mentions and images never use a composer, send button, file input, conversation click, message scroll or intercepted send template. Verify HTTP status, server result, client ID echo and selected conversation. Receipts retain only redacted endpoint metadata and hashes; do not claim a send from visible DOM text.

## Diagnose Drift

When send field construction or dynamic request context has changed, an optional headed capture can record a user-performed request for research:

```text
node <skill-directory>/bin/doudou.js protocol capture --profile <name>
```

Tell the user to perform one normal send or reference reply in the headed browser and close that browser when finished. Inspect the generated manifest and binary bodies under `message/profiles/<profile>/diagnostics/captures/`. Headers and sensitive query values are redacted in the manifest.

For page or bundle drift, run `page-doctor --profile <name>` and report the diagnostic directory and field-level findings.

## Recovery Rules

- On `PROFILE_IN_USE`, show the redacted owner and obtain confirmation before retrying with `--terminate-owner`.
- On `FINGERPRINT_BASELINE_REQUIRED`, run the returned headed calibration command, then resume once.
- On `LOGIN_REQUIRED` or `VERIFICATION_REQUIRED`, run headed `profile login`, then retry the original headless command once.
- A failed live warmup may use the newest successful snapshot for at most two hours for lookup, collection, and confirmation previews. The output is marked `warmup_source: stale_cache`. Confirmed protocol writes always require a new successful live warmup.
- Keep project-root `profile/` and `message/` local and ignored. A legacy `Profile/` root is migrated once. Never copy either directory into the Skill folder or a delivered archive.
