# Runtime layout and file naming

All paths are relative to the active data root. The default data root is the
caller's current project directory, not the installed `douyin-message/` Skill
directory. Override it with `--data-root PATH` or `DOUYIN_MESSAGE_HOME`.

The CLI creates missing directories. A new user starts with only the installed
Skill; `doctor`, `profile create`, conversation refresh, collection and sending
create the project layout as each capability is first used.

## Login Profile

```text
profile/<profile>/profile.json
profile/<profile>/fingerprint.json
profile/<profile>/browser-data/
profile/<profile>/.lock.json
```

`doctor` creates `profile/`. Existing `Profile/` is migrated once when
`profile/` is absent. `profile create` creates metadata and
`browser-data/`. Headed `profile login` creates the browser session and
fingerprint baseline.

## Conversations and Messages

```text
message/profiles/<profile>/conversations.json
message/profiles/<profile>/warmup/latest.json
message/profiles/<profile>/warmup/YYYY-MM-DD/YYYYMMDDTHHMMSSmmmZ__warmup.json
message/profiles/<profile>/conversations/<kind>__<display-name>__<conversation-id>/conversation.json
message/profiles/<profile>/conversations/<kind>__<display-name>__<conversation-id>/messages.jsonl
message/profiles/<profile>/conversations/<kind>__<display-name>__<conversation-id>/conflicts.jsonl
message/profiles/<profile>/runs/run-<UTC-basic-time>-<id>.json
```

`doctor` creates `message/`. Conversation refresh creates the profile index.
Warmup writes a redacted current snapshot and dated history. Snapshots contain
conversation/user mappings and bounded DOM/network contract counts and paths,
but no Cookie, token, dynamic signature or complete sensitive query string.
Collection creates conversation folders and Run records. `kind` is `group` or
`direct`; unsafe path characters are deterministically encoded. Conversation
and message IDs remain decimal strings.

## Sends

```text
message/profiles/<profile>/sends/YYYY-MM-DD/YYYYMMDDTHHMMSSmmmZ__<client-message-id>/
  01-sdk-signed-request.bin
  02-cli-request.bin
  03-server-response.bin
  04-receipt.json
  05-verification.json
```

The send command creates the dated folder only after intercepting a fresh
page-signed request. The numbered names define causal order. The receipt records
conversation, mode, IDs, endpoint, response and SHA-256 hashes. Verification
contains literal Boolean checks and the final conclusion.

## Optional Diagnostics

```text
message/profiles/<profile>/diagnostics/captures/YYYY-MM-DD/YYYYMMDDTHHMMSSmmmZ__capture-<UTC-basic-time>-<id>/
  manifest.json
  bodies/NNNN-request.bin
  bodies/NNNN-response.bin
message/profiles/<profile>/diagnostics/page-doctor/YYYY-MM-DD/doctor-<UTC-basic-time>-<id>/
```

Capture manifests redact sensitive URL and header values. Diagnostics remain
with the same project and Profile and are never written inside the Skill.

## Delivery Boundary

Project `profile/` and `message/` must be ignored by version control. The
deliverable is the installed `douyin-message/` folder selected by
`package.json.files`. Never copy project runtime directories into it.
