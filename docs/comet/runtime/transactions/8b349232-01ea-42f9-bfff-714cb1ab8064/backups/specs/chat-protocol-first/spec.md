# Chat Protocol First

## Purpose

Douyin Message SHALL operate its IM workflow from `/chat` and SHALL use the current page's authenticated protocol context for message reads and writes.

## Entry and discovery

- Every headless warmup, conversation refresh, collection, send and reply workflow SHALL navigate directly to `https://www.douyin.com/chat`.
- The implementation SHALL NOT navigate to the site homepage or click a homepage message entry as part of those workflows.
- Conversation classification SHALL use the decoded Protobuf `conversation_type` as the primary source: `1` means direct and `2` means group. Bounded explicit page attributes may supplement missing structured data but SHALL NOT override it.

## Read protocol

- The implementation SHALL capture the current `/chat` page's initialization request context and perform supported IM read calls through `page.evaluate(fetch(...))` with browser credentials.
- A read workflow SHALL NOT issue mark-read or other write endpoints merely to collect a conversation.
- Read captures and diagnostics SHALL contain only endpoint paths, parameter names, byte counts, field shapes and hashes; they SHALL NOT persist raw message bodies, raw identifiers, signed URLs, cookies or tokens.

## Send protocol

- Before a confirmed plain send or reference reply, the implementation SHALL create a fresh SDK request template within the current `/chat` page.
- The template request SHALL be intercepted and aborted before it reaches the network.
- The implementation SHALL build the final Protobuf body from that template and SHALL submit it with `fetch` in the page context using the original, in-memory signed URL.
- The implementation SHALL verify HTTP success, Protobuf status, server result, client message ID and selected conversation before reporting `sent`.
- The command SHALL return a stable failure without sending a fallback message when a fresh template cannot be captured or the receipt verification fails.

## Artifacts and privacy

- Send receipts and verification records SHALL exclude complete signed URLs, cookies, tokens and raw identifiers. They MAY include redacted endpoint metadata and SHA-256 values.
- Project-local `profile/`, `message/` and `analyze/` data SHALL remain outside the delivered Skill and Git-tracked source.

## Compatibility

- Existing profile setup, environment doctor, warmup freshness rules, confirmation plans, plain sends and reference replies SHALL remain available for both direct and group conversations.
