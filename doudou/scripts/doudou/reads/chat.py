from __future__ import annotations

import json
from datetime import datetime, timezone
from pathlib import Path
from typing import Any

from ..common.core import random_id, utc_now
from ..common.errors import DoudouError
from ..models.identity import (
    IdentityCache,
    enrich_message_identities,
    known_identity_sec_uids,
    normalize_identity,
)
from ..models.resolver import infer_self_uid, resolve_conversation
from ..protocol.contracts import WEB_PROTOCOL
from ..protocol.im import (
    build_command_request,
    extract_messages,
    find_conversations,
    messages_request,
    parse_participants_page,
    parse_stranger_conversations,
    parse_stranger_messages,
    participants_request,
    response_body,
    stranger_conversations_request,
    stranger_messages_request,
    stranger_request,
    stranger_response_body,
)
from ..protocol.protobuf import decode_fields, get_field
from ..protocol.responses import ResponseCodes
from ..runtime.browser.session import BrowserSession
from ..runtime.paths import profile_paths
from ..storage.media import download_message_images
from ..storage.repository import (
    merge_messages,
    read_conversation_index,
    read_messages,
    read_session_index_snapshot,
    write_conversation_index,
    write_session_index_snapshot,
)
from .transport import parse_result

INDEX_MAX_AGE_SECONDS = 2 * 60 * 60
GET_MESSAGES_ENDPOINT = "https://imapi.douyin.com/v1/message/get_by_conversation"
PARTICIPANTS_ENDPOINT = "https://imapi.douyin.com/v1/conversation/participants_list"
STRANGER_CONVERSATIONS_ENDPOINT = "https://imapi.douyin.com/v1/stranger/get_conversation_list"
STRANGER_MESSAGES_ENDPOINT = "https://imapi.douyin.com/v1/stranger/get_messages"


def _response_identities(value: Any) -> list[dict[str, Any]]:
    found: list[dict[str, Any]] = []
    pending = [(value, 0)]
    while pending:
        current, depth = pending.pop()
        if depth > 6:
            continue
        if isinstance(current, dict):
            identity = normalize_identity(current, source="user-info")
            if identity.get("uid") or identity.get("sec_uid"):
                found.append(identity)
            pending.extend((child, depth + 1) for child in current.values())
        elif isinstance(current, list):
            pending.extend((child, depth + 1) for child in current)
    return found


async def _resolve_user_identities(
    session: BrowserSession,
    data_root: Path,
    profile: str,
    sec_uids: list[str],
) -> dict[str, dict[str, Any]]:
    identities: dict[str, dict[str, Any]] = {}
    cache = IdentityCache(data_root, profile)
    unique = list(dict.fromkeys(value for value in sec_uids if value))
    captured_responses = await session.captured_user_info_responses(
        timeout_seconds=1.5, quiet_seconds=0.25
    )
    for captured in captured_responses:
        try:
            value = json.loads(captured)
        except (UnicodeDecodeError, json.JSONDecodeError):
            value = None
        if not isinstance(value, dict) or int(value.get("status_code") or 0) != 0:
            continue
        for identity in _response_identities(value):
            sec_uid = identity.get("sec_uid")
            if not sec_uid or sec_uid not in unique:
                continue
            merged = cache.merge(identity, source="page-user-info")
            identities[sec_uid] = merged
    missing = [sec_uid for sec_uid in unique if sec_uid not in identities]
    endpoint = await session.captured_user_info_endpoint()
    for offset in range(0, len(missing), 20):
        result = await session.page.evaluate(
            """async ({values, endpoint}) => {
              const body = new URLSearchParams({sec_user_ids: JSON.stringify(values)});
              const response = await fetch(endpoint, {method: 'POST', credentials: 'include', headers: {'content-type': 'application/x-www-form-urlencoded'}, body});
              return {http_status: response.status, content_type: response.headers.get('content-type'), retry_after: response.headers.get('retry-after'), text: await response.text()};
            }""",
            {
                "values": missing[offset : offset + 20],
                "endpoint": endpoint or WEB_PROTOCOL.path("user_info"),
            },
        )
        try:
            value = await parse_result(
                session,
                result,
                codes=ResponseCodes(
                    "USER_INFO_RESPONSE_EMPTY",
                    "USER_INFO_RESPONSE_INVALID",
                    "USER_INFO_LOOKUP_FAILED",
                ),
                operation="chat.user_info",
            )
        except DoudouError as error:
            if error.code in {"LOGIN_REQUIRED", "VERIFICATION_REQUIRED"}:
                raise
            continue
        for identity in _response_identities(value):
            sec_uid = identity.get("sec_uid")
            if not sec_uid:
                continue
            merged = cache.merge(identity, source="user-info")
            identities[sec_uid] = merged
    for sec_uid in unique:
        if sec_uid not in identities:
            identities[sec_uid] = cache.enrich(
                {"sec_uid": sec_uid}, source="user-info-cache"
            )
    cache.save()
    return identities


async def _enrich_conversation_identities(
    session: BrowserSession,
    data_root: Path,
    profile: str,
    conversations: list[dict[str, Any]],
) -> None:
    self_uid = infer_self_uid(conversations)
    targets = [
        participant
        for conversation in conversations
        for participant in conversation.get("participants", {}).get("participants", [])
        if participant.get("sec_uid")
    ]
    targets.extend(
        (conversation.get("last_message") or {}).get("sender") or {}
        for conversation in conversations
        if ((conversation.get("last_message") or {}).get("sender") or {}).get(
            "sec_uid"
        )
    )
    identities = await _resolve_user_identities(
        session, data_root, profile, [item["sec_uid"] for item in targets]
    )
    for conversation in conversations:
        participants = conversation.get("participants", {}).get("participants", [])
        for index, participant in enumerate(participants):
            resolved = identities.get(participant.get("sec_uid") or "")
            if resolved:
                participants[index] = {**participant, **resolved}
        last_message = conversation.get("last_message")
        if isinstance(last_message, dict):
            sender = last_message.get("sender") or {}
            resolved = identities.get(sender.get("sec_uid") or "")
            if resolved:
                last_message["sender"] = {**sender, **resolved}
            last_message["sender"]["name"] = last_message["sender"].get(
                "display_name"
            )
        if conversation.get("conversation_type", 1) != 1 and conversation.get("kind") != "direct":
            continue
        # Direct conversations should be addressable by the other participant,
        # while never replacing the title with the current account's identity.
        # If self cannot be inferred, keep the protocol title instead of
        # guessing from the last sender, which may be the current account.
        others = [
            participant
            for participant in participants
            if self_uid and str(participant.get("uid") or "") != self_uid
        ]
        names = [
            participant.get("remark_name")
            or participant.get("nickname")
            or participant.get("display_name")
            or participant.get("alias")
            for participant in others
        ]
        names = [str(value).strip() for value in names if value and str(value).strip()]
        if len(names) == 1:
            conversation["display_name"] = names[0]


def _age_seconds(timestamp: str | None) -> float | None:
    if not timestamp:
        return None
    try:
        value = datetime.fromisoformat(timestamp.replace("Z", "+00:00"))
    except ValueError:
        return None
    return max(
        0.0,
        (datetime.now(timezone.utc) - value.astimezone(timezone.utc)).total_seconds(),
    )


async def refresh_session_index(
    session: BrowserSession, data_root: Path, profile: str, force: bool = False
) -> dict[str, Any]:
    cached = read_session_index_snapshot(data_root, profile)
    age = _age_seconds(cached.get("completed_at") if isinstance(cached, dict) else None)
    if (
        not force
        and cached
        and "inbox_count" in cached
        and age is not None
        and age <= INDEX_MAX_AGE_SECONDS
    ):
        await session.ensure_ready()
        index = read_conversation_index(data_root, profile)
        if index:
            return {
                "source": "cache",
                "age_seconds": age,
                "snapshot": cached,
                "index": index,
            }
    ready = await session.ensure_ready()
    if not session.init_response:
        raise DoudouError(
            "VERIFICATION_REQUIRED",
            "IM initialization did not produce a usable response.",
        )
    conversations = find_conversations(session.init_response)

    # Message requests use a separate command and are merged only after every
    # page succeeds, preserving the previous index on protocol failure.
    if not session.init_request:
        raise DoudouError("IM_INITIALIZATION_MISSING", "IM request envelope is missing.")
    stranger_by_id: dict[str, dict[str, Any]] = {}
    cursor, seen = 0, set()
    for _ in range(100):
        if cursor in seen:
            raise DoudouError("STRANGER_PAGE_CURSOR_STALLED", "Stranger inbox cursor did not advance.")
        seen.add(cursor)
        request = stranger_request(
            session.init_request, 1001, stranger_conversations_request(cursor, 50, True)
        )
        http_status, raw = await session.post_protobuf(STRANGER_CONVERSATIONS_ENDPOINT, request)
        status, status_text, payload, gateway = stranger_response_body(raw, 1001)
        if gateway is not None:
            raise DoudouError("VERIFICATION_REQUIRED", "Stranger conversation request reached a verification gateway.")
        if http_status != 200 or status != 0 or payload is None:
            raise DoudouError("STRANGER_CONVERSATIONS_FAILED", "Stranger conversation protocol request failed.", {"http_status": http_status, "status": status, "status_text": status_text})
        page = parse_stranger_conversations(payload)
        for item in page["conversations"]:
            stranger_by_id[str(item["conversation_id"])] = item
        next_cursor = str(page.get("next_cursor") or "0")
        if not page.get("has_more"):
            break
        if next_cursor == str(cursor):
            raise DoudouError(
                "STRANGER_PAGE_CURSOR_STALLED",
                "Stranger inbox cursor did not advance.",
                {"cursor": str(cursor)},
            )
        try:
            cursor = int(next_cursor)
        except ValueError as error:
            raise DoudouError(
                "PROTOCOL_SCHEMA_DRIFTED",
                "Stranger inbox returned a non-numeric cursor.",
                {"cursor": next_cursor},
            ) from error
    else:
        raise DoudouError("STRANGER_PAGE_LIMIT", "Stranger conversation collection reached the page limit.")
    stranger = list(stranger_by_id.values())
    conversations.extend(stranger)
    if not conversations:
        raise DoudouError(
            "IM_INITIALIZATION_EMPTY",
            "IM initialization and stranger inbox returned no conversations.",
        )
    await _enrich_conversation_identities(session, data_root, profile, conversations)
    index = write_conversation_index(data_root, profile, conversations)
    snapshot = {
        "schema": "doudou.session-index-snapshot.v1",
        "profile_id": profile_paths(data_root, profile).profile_id,
        "completed_at": utc_now(),
        "conversation_count": len(conversations),
        "direct_count": sum(item.get("kind") == "direct" for item in conversations),
        "group_count": sum(item.get("kind") == "group" for item in conversations),
        "inbox_count": sum(item.get("inbox") == "request" for item in conversations),
        "browser_initialization_count": ready["initialization_count"],
    }
    files = write_session_index_snapshot(data_root, profile, snapshot)
    return {
        "source": "live",
        "age_seconds": 0.0,
        "snapshot": snapshot,
        "index": index,
        "files": files,
    }


async def collect_messages(
    session: BrowserSession,
    data_root: Path,
    profile: str,
    target: str,
    from_time: str | None = None,
    to_time: str | None = None,
    max_pages: int = 100,
    download_images: bool = True,
) -> dict[str, Any]:
    refreshed = await refresh_session_index(session, data_root, profile)
    conversation = resolve_conversation(refreshed["index"]["conversations"], target)
    if not session.init_request:
        raise DoudouError(
            "IM_INITIALIZATION_MISSING",
            "The current broker session has no IM request envelope.",
        )
    lower = (
        datetime.fromisoformat(from_time.replace("Z", "+00:00")).timestamp() * 1000
        if from_time
        else None
    )
    upper = (
        datetime.fromisoformat(to_time.replace("Z", "+00:00")).timestamp() * 1000
        if to_time
        else None
    )
    messages: dict[str, dict[str, Any]] = {}
    pages: list[dict[str, Any]] = []
    anchor = 0
    for page_number in range(1, max_pages + 1):
        if conversation.get("inbox") == "request":
            body = stranger_messages_request(
                conversation.get("conversation_short_id") or conversation["conversation_id"],
                reset_unread_count=False,
            )
            request = stranger_request(session.init_request, 1002, body)
            endpoint, command = STRANGER_MESSAGES_ENDPOINT, 1002
        else:
            body = messages_request(conversation, anchor_index=anchor)
            request = build_command_request(session.init_request, 301, body)
            endpoint, command = GET_MESSAGES_ENDPOINT, 301
        http_status, raw = await session.post_protobuf(endpoint, request)
        status, status_text, payload, gateway = (
            stranger_response_body(raw, command)
            if command == 1002
            else response_body(raw, command)
        )
        if gateway is not None:
            raise DoudouError(
                "VERIFICATION_REQUIRED",
                "Message collection reached a JSON verification gateway.",
                {"gateway_keys": sorted(gateway)},
            )
        if http_status != 200 or status != 0 or payload is None:
            raise DoudouError(
                "MESSAGE_COLLECT_FAILED",
                "Message protocol request failed.",
                {
                    "http_status": http_status,
                    "status": status,
                    "status_text": status_text,
                },
            )
        batch = (
            parse_stranger_messages(payload, conversation["conversation_id"])
            if command == 1002
            else extract_messages(payload, conversation["conversation_id"])
        )
        protocol_received = len(batch)
        summary_received = 0
        collection_source = "protocol" if batch else "empty"
        if command == 1002 and not batch:
            summary = conversation.get("last_message")
            if isinstance(summary, dict) and summary.get("message_id"):
                batch = [
                    {
                        **summary,
                        "sender": dict(summary.get("sender") or {}),
                        "collection_source": "stranger-conversation-summary",
                    }
                ]
                summary_received = 1
                collection_source = "conversation_summary"
        for item in batch:
            created = int(item["create_time_ms"])
            if (lower is None or created >= lower) and (
                upper is None or created <= upper
            ):
                messages[item["message_id"]] = item
        if command == 1002:
            next_cursor, has_more = 0, False
        else:
            fields = decode_fields(payload)
            next_field, more_field = get_field(fields, 2, 0), get_field(fields, 3, 0)
            next_cursor = int(next_field.value) if next_field else 0
            has_more = bool(int(more_field.value)) if more_field else False
        pages.append(
            {
                "page": page_number,
                "anchor_in": str(anchor),
                "next_cursor": str(next_cursor),
                "received": len(batch),
                "protocol_received": protocol_received,
                "summary_received": summary_received,
                "collection_source": collection_source,
                "fallback_used": False,
                "protocol_http_status": http_status,
                "protocol_status": status,
                "payload_bytes": len(payload),
                "has_more": has_more,
            }
        )
        oldest = min((int(item["create_time_ms"]) for item in batch), default=None)
        if (
            not has_more
            or next_cursor == anchor
            or (lower is not None and oldest is not None and oldest < lower)
        ):
            break
        anchor = next_cursor
    else:
        raise DoudouError(
            "MESSAGE_PAGE_LIMIT",
            "Message collection reached the configured page limit.",
            {"max_pages": max_pages},
        )

    ordered = sorted(
        messages.values(),
        key=lambda item: (item["create_time_ms"], int(item["message_id"])),
    )
    identities = await _resolve_user_identities(
        session,
        data_root,
        profile,
        known_identity_sec_uids(ordered),
    )
    for message in ordered:
        sender = message.get("sender") or {}
        resolved = identities.get(sender.get("sec_uid") or "")
        if resolved:
            sender = {**sender, **resolved}
        sender["name"] = sender.get("display_name")
        message["sender"] = sender
    last_message = conversation.get("last_message")
    if isinstance(last_message, dict):
        sender = last_message.get("sender") or {}
        resolved = identities.get(sender.get("sec_uid") or "")
        if resolved:
            conversation["last_message"]["sender"] = {**sender, **resolved}
        conversation["last_message"]["sender"]["name"] = conversation[
            "last_message"
        ]["sender"].get("display_name")
    cache = IdentityCache(data_root, profile)
    ordered = enrich_message_identities(
        ordered,
        cache,
        source="message-collect",
        reference_catalog=read_messages(data_root, profile, conversation),
    )
    cache.save()
    image_stats = {"attempted": 0, "downloaded": 0, "failed": 0}
    if download_images:
        paths = profile_paths(data_root, profile)
        for message in ordered:
            if message["message_type"] != 27:
                continue
            day = (
                datetime.fromtimestamp(message["create_time_ms"] / 1000, timezone.utc)
                .date()
                .isoformat()
            )
            directory = paths.conversations / "media" / day / message["message_id"]
            results = await __import__("asyncio").to_thread(
                download_message_images, message, directory
            )
            message["images"] = results
            image_stats["attempted"] += len(results)
            image_stats["downloaded"] += sum(
                item["status"] == "downloaded" for item in results
            )
            image_stats["failed"] += sum(
                item["status"] != "downloaded" for item in results
            )
    merge = merge_messages(data_root, profile, conversation, ordered)
    inventory: dict[str, int] = {}
    for message in ordered:
        for field in message.get("unknown_fields") or []:
            key = f"field_{field['field']}_wire_{field['wire']}"
            inventory[key] = inventory.get(key, 0) + 1
    run = {
        "schema": "doudou.message-collect-run.v1",
        "run_id": random_id("collect"),
        "completed_at": utc_now(),
        "conversation": conversation,
        "pages": pages,
        "message_count": len(ordered),
        "unknown_field_inventory": inventory,
        "images": image_stats,
        "merge": merge,
    }
    return run


async def resolve_members(
    session: BrowserSession,
    data_root: Path,
    profile: str,
    conversation: dict[str, Any],
    requested: list[str],
) -> list[dict[str, Any]]:
    if not requested:
        return []
    if conversation["conversation_type"] != 2:
        raise DoudouError(
            "MENTION_REQUIRES_GROUP", "Mentions are only valid in group conversations."
        )
    if not session.init_request:
        raise DoudouError(
            "IM_INITIALIZATION_MISSING",
            "The current broker session has no IM request envelope.",
        )
    initial = conversation.get("participants") or {
        "participants": [],
        "has_more": False,
        "cursor": "0",
    }
    members = list(initial.get("participants") or [])
    cursor, has_more = initial.get("cursor") or "0", bool(initial.get("has_more"))
    for _ in range(100):
        if not has_more:
            break
        request = build_command_request(
            session.init_request, 605, participants_request(conversation, cursor)
        )
        http_status, raw = await session.post_protobuf(PARTICIPANTS_ENDPOINT, request)
        status, status_text, payload, gateway = response_body(raw, 605)
        if gateway is not None:
            raise DoudouError(
                "VERIFICATION_REQUIRED",
                "Member lookup reached a JSON verification gateway.",
            )
        if http_status != 200 or status != 0 or payload is None:
            raise DoudouError(
                "PARTICIPANT_LIST_FAILED",
                "Group member protocol request failed.",
                {
                    "http_status": http_status,
                    "status": status,
                    "status_text": status_text,
                },
            )
        wrapper = decode_fields(payload)
        page_field = get_field(wrapper, 1, 2)
        page = parse_participants_page(bytes(page_field.value) if page_field else b"")
        members.extend(page["participants"])
        cursor, has_more = page["cursor"], page["has_more"]
    unique = {item["uid"]: item for item in members}
    members = list(unique.values())
    identities = await _resolve_user_identities(
        session,
        data_root=data_root,
        profile=profile,
        sec_uids=[item["sec_uid"] for item in members if item.get("sec_uid")],
    )
    for member in members:
        resolved = identities.get(member.get("sec_uid") or "")
        if resolved:
            member.update(resolved)
        member["name"] = member.get("display_name") or member.get("alias") or ""
    resolved: list[dict[str, Any]] = []
    for query in requested:
        matches = [
            member
            for member in members
            if query
            in {
                member.get("name"),
                member.get("alias"),
                member.get("uid"),
                member.get("sec_uid"),
            }
        ]
        if not matches:
            raise DoudouError(
                "MENTION_MEMBER_NOT_FOUND",
                "Group member was not found.",
                {"query": query},
            )
        if len(matches) > 1:
            raise DoudouError(
                "MENTION_MEMBER_AMBIGUOUS",
                "Group member name is ambiguous.",
                {
                    "query": query,
                    "candidates": [
                        {"uid": item["uid"], "name": item["name"]} for item in matches
                    ],
                },
            )
        resolved.append(matches[0])
    return list({item["uid"]: item for item in resolved}.values())
