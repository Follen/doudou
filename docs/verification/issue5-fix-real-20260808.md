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

## Modified 1002 Collection

Command:

```powershell
python doudou\scripts\doudou.py --data-root D:\Code\doudou messages collect --profile PROFILE_REDACTED --conversation COUNTERPARTY_NICKNAME --from 2000-01-01T00:00:00Z --to 2100-01-01T00:00:00Z
```

Sanitized result:

```text
EXIT=0 message_count=0 page_protocol_received=0 page_received=0 fallback_used=False http_status=200 protocol_status=0 payload_bytes=0 stored_total=0 added=0 duplicates=0
```

This is a real non-empty request conversation whose 1002 response carried an empty body. The result no longer copies 1001 `last_message` into `messages.jsonl`.

## Automated Verification

```text
PYTHONPATH=doudou\scripts python -m pytest docs\verification\tests -q
164 passed, 6 subtests passed in 39.90s

PYTHONPATH=doudou\scripts python -m pytest docs\verification\tests\test_stranger_chat_integration.py docs\verification\tests\test_identity.py docs\verification\tests\test_broker.py -q
57 passed, 6 subtests passed in 29.03s

python -m ruff check doudou\scripts docs\verification\tests
All checks passed!
```

## Rollback

After the change commit is created, rollback is:

```powershell
git revert <ISSUE5_FIX_COMMIT>
```

The rollback preserves the pre-change files and can be verified with the baseline refresh command above.
