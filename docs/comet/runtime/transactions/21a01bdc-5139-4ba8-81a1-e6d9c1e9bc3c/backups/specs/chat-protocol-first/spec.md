# Chat Protocol First

## Purpose

Douyin Message SHALL operate its IM workflow from `/chat` and SHALL use the current page's authenticated protocol context for message reads and writes.

## Entry and discovery

- Every headless warmup, conversation refresh, collection, send and reply workflow SHALL navigate directly to `https://www.douyin.com/chat`.
- The implementation SHALL NOT navigate to the site homepage or click a homepage message entry as part of those workflows.
- Conversation classification SHALL use the decoded Protobuf `conversation_type` as the primary source: `1` means direct and `2` means group. Bounded explicit page attributes may supplement missing structured data but SHALL NOT override it.
- Message collection and send workflows SHALL NOT open a conversation through DOM interaction or inspect React-owned objects.

## Read protocol

- The implementation SHALL capture the current `/chat` page's initialization request context and perform supported IM read calls through `page.evaluate(fetch(...))` with browser credentials.
- A read workflow SHALL NOT issue mark-read or other write endpoints merely to collect a conversation.
- Read captures and diagnostics SHALL contain only endpoint paths, parameter names, byte counts, field shapes and hashes; they SHALL NOT persist raw message bodies, raw identifiers, signed URLs, cookies or tokens.
- Image messages SHALL be decoded from Protobuf content. Their encrypted resource SHALL be downloaded directly, decrypted with AES-256-GCM using the message `skey`, verified against declared length and top-level content MD5, and written atomically to the project-local conversation media directory.
- A media failure SHALL be recorded per image and remain retryable without discarding otherwise valid message records or stopping unrelated conversations.

## Send protocol

- Before a confirmed plain send, reference reply, mention send or image send, the implementation SHALL construct the required Protobuf body directly without using a composer, send button, file input, synthetic keyboard event or intercepted SDK send template.
- The implementation SHALL submit the final Protobuf body with `fetch` in the `/chat` page context so the current browser security layer supplies Cookie and dynamic request parameters.
- The implementation SHALL verify HTTP success, Protobuf status, server result, client message ID and selected conversation before reporting `sent`.
- The command SHALL return a stable failure without sending a DOM fallback message when protocol initialization, request submission or receipt verification fails.
- Mention sends SHALL resolve members from protocol metadata and SHALL encode UTF-16 text ranges, UID metadata and matching bold metadata; mentions SHALL be rejected for direct conversations.
- Image sends SHALL obtain upload credentials through the authenticated page request context, perform the VOD Apply/TOS upload/VOD Commit protocol, call `batch_build_image` through page-context fetch, then send `message_type=27` with `aweType=2702`.

## Artifacts and privacy

- Send receipts and verification records SHALL exclude complete signed URLs, cookies, tokens and raw identifiers. They MAY include redacted endpoint metadata and SHA-256 values.
- Media records SHALL use project-relative paths. Temporary credentials, TOS Auth values, encryption keys and complete signed resource URLs SHALL NOT be copied into receipts or diagnostics.
- Project-local `profile/`, `message/` and `analyze/` data SHALL remain outside the delivered Skill and Git-tracked source.

## Compatibility

- Existing profile setup, environment doctor, warmup freshness rules, confirmation plans, plain sends and reference replies SHALL remain available for both direct and group conversations.
- Batch send SHALL remain strictly sequential and SHALL preserve the five-second delay between successful sends.
