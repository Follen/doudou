# Issue #5 Real Verification

Date: 2026-08-08
Source: `0.0.7` baseline versus `codex/issue5-fix` working tree

All account, conversation, message, cookie, Profile, and sec_uid values are redacted. The record keeps only counts, statuses, timings, and protocol byte counts.

## Baseline

Command (executed from `D:\Code\doudou`, npm launcher, `--profile PROFILE_REDACTED`):

```powershell
node npm\bin\doudou.mjs conversations refresh --profile PROFILE_REDACTED --kind direct --inbox request --force
```

Five consecutive live refreshes on the logged-in profile:

```text
RUN=1 EXIT=0 ELAPSED_MS=1755
RUN=2 EXIT=0 ELAPSED_MS=1794
RUN=3 EXIT=0 ELAPSED_MS=1663
RUN=4 EXIT=0 ELAPSED_MS=1690
RUN=5 EXIT=1 ELAPSED_MS=1604
BROKER_INTERNAL_ERROR: Page.evaluate: NetworkError when attempting to fetch resource.
```

This reproduces the reported transient failure.

## Modified Refresh

Command (executed from `D:\Code\issue5-fix`, current Python source, `--data-root D:\Code\doudou`, `--profile PROFILE_REDACTED`):

```powershell
$env:PYTHONPATH=(Resolve-Path 'doudou\scripts').Path
python doudou\scripts\doudou.py --data-root D:\Code\doudou conversations refresh --profile PROFILE_REDACTED --kind direct --inbox request --force
```

Five consecutive live refreshes after the fix:

```text
RUN=1 EXIT=0 ELAPSED_MS=14581 count=1;title_is_id=False;participants=2
RUN=2 EXIT=0 ELAPSED_MS=1709 count=1;title_is_id=False;participants=2
RUN=3 EXIT=0 ELAPSED_MS=2138 count=1;title_is_id=False;participants=2
RUN=4 EXIT=0 ELAPSED_MS=1746 count=1;title_is_id=False;participants=2
RUN=5 EXIT=0 ELAPSED_MS=2384 count=1;title_is_id=False;participants=2
```

The live request conversation resolved successfully by the redacted counterparty nickname; the returned title was not the conversation ID.

## Browser Network Capture

The current logged-in web client was opened with the project Camoufox Profile. After clicking the visible stranger inbox and the request conversation, sanitized network capture showed:

```text
clicked_request_portal=true
clicked_conversation=true
native_1002_requests=0
native_1001_requests>0
native_1006_mark_read_requests>0
last_message_nonempty=true
last_message_visible_in_dom=true
```

The web page renders the non-empty 1001 `last_message`; it does not request 1002 for this interaction. No captured request body, response body, account value, conversation ID, sec_uid, nickname, or message text is stored in this report.

## Modified Collection And Persistence

Command:

```powershell
python doudou\scripts\doudou.py --data-root D:\Code\doudou messages collect --profile PROFILE_REDACTED --conversation COUNTERPARTY_NICKNAME --from 2000-01-01T00:00:00Z --to 2100-01-01T00:00:00Z
```

Two consecutive sanitized results:

```text
RUN=1 EXIT=0 message_count=1 protocol_received=0 summary_received=1 collection_source=conversation_summary fallback_used=False http_status=200 protocol_status=0 payload_bytes=0 total=1 added=1 duplicates=0 stored_source=stranger-conversation-summary
RUN=2 EXIT=0 message_count=1 protocol_received=0 summary_received=1 collection_source=conversation_summary fallback_used=False http_status=200 protocol_status=0 payload_bytes=0 total=1 added=0 duplicates=1 stored_source=stranger-conversation-summary
```

The stored message text hash matched the visible 1001 `last_message` hash on both runs. The second run kept the total at one and reported one duplicate, proving incremental deduplication instead of overwrite. The summary is explicitly labelled and is not counted as a 1002 protocol message.

## Automated Verification

```text
PYTHONPATH=doudou\scripts python -m pytest docs\verification\tests -q
166 passed, 6 subtests passed in 32.49s

PYTHONPATH=doudou\scripts python -m pytest docs\verification\tests\test_stranger_chat_integration.py docs\verification\tests\test_identity.py docs\verification\tests\test_broker.py -q
59 passed, 6 subtests passed in 30.77s

python -m ruff check doudou\scripts docs\verification\tests
All checks passed!
```

## Rollback

After the change commit is created, rollback is:

```powershell
git revert <ISSUE5_FIX_COMMIT>
```

The rollback preserves the pre-change files and can be verified with the baseline refresh command above.
