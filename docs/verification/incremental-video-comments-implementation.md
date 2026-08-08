# Incremental Video Comments Implementation

## Code

- `doudou/scripts/doudou/storage/content.py`: `merge_video_comments` provides CID uniqueness, stable historical order, recursive object merge, blank/null preservation, and meaningful `0`/`false` updates.
- `doudou/scripts/doudou/reads/video.py`: current-run checkpoints stay separate from the final merged collection; prior formal comments are merged only at successful commit; reply-template resolution has bounded retries and a root-template fallback.
- `doudou/scripts/doudou/ui/components/video.py`: the unique visible scrollable `.comment-mainContent` is preferred, including an ancestor container; reply-trigger clicks have bounded retries.
- Tests cover 600-to-400 retention, 600-plus-20 growth, recursive field merge, falsy values, duplicate CID rows, reply pagination, cursor stalls, page limits, scroll-root selection, and bounded retries.
- `doudou/references/runtime.md` documents incremental storage and protocol completion.

## Real Browser Verification

- Profile: `Follen-doudou` (existing valid login)
- Video: `7667910062194609417`
- Baseline: 462 unique CID, 305 roots, 157 replies, 32 root pages, 48 reply pages, 2 multi-page reply threads.
- Command: local `doudou/scripts/doudou.py video collect` using managed Python and local source `PYTHONPATH`.
- Result: exit `0`; final formal file 479 unique rows; 0 duplicate rows; 0 historical CID missing; 17 new CID; 32 root pages terminal; 49 reply threads terminal; completed checkpoint 478 rows (current run only).
- Baseline `comments.jsonl` SHA-256: `f37eaffcbd49f3187f2b2f2fef26d3bbd30c2ce016bf62e8ac318f388d804870`.
- Modified `comments.jsonl` SHA-256: `2dc9ce30e8069b89842d2488eec1d988fd6b23befc06905d0c95d5ef239d1a0d`.
- Real DOM probe: two `.comment-mainContent` nodes existed; one was visible and scrollable, and the component selected it and moved `scrollTop` from `0` to `718`.
- Local rollback backup: `analyze/incremental-video-comments/real-20260808T045114/7667910062194609417/`.
