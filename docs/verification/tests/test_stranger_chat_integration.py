from __future__ import annotations

import asyncio
import json

import pytest
from doudou.common.errors import DoudouError
from doudou.features.chat import service
from doudou.protocol.protobuf import decode_fields, get_field
from doudou.reads import chat
from doudou.storage.repository import write_conversation_index


class Session:
    init_request = b"init-request"
    init_response = b"init-response"

    async def ensure_ready(self):
        return {"initialization_count": 1}


def established():
    return {
        "conversation_id": "known",
        "conversation_short_id": "10",
        "conversation_type": 1,
        "kind": "direct",
        "display_name": "Known",
        "participants": {"participants": [], "has_more": False, "cursor": "0"},
    }


def stranger():
    return {
        "conversation_id": "request",
        "conversation_short_id": "20",
        "conversation_type": 1,
        "kind": "direct",
        "inbox": "request",
        "display_name": "Request",
        "participants": {"participants": [], "has_more": False, "cursor": "0"},
        "last_message": {
            "message_id": "last",
            "conversation_id": "request",
            "create_time_ms": 1,
            "message_type": 7,
            "sender": {"sec_uid": "sec-request"},
            "unknown_fields": [],
        },
    }


def test_refresh_failure_preserves_previous_index(tmp_path, monkeypatch):
    previous = write_conversation_index(tmp_path, "work", [established()])
    session = Session()

    async def post_protobuf(endpoint, request):
        return 500, b"failed"

    session.post_protobuf = post_protobuf
    monkeypatch.setattr(chat, "find_conversations", lambda raw: [established()])

    async def no_enrichment(*args):
        return None

    monkeypatch.setattr(chat, "_enrich_conversation_identities", no_enrichment)
    monkeypatch.setattr(chat, "stranger_request", lambda *args: b"request")
    monkeypatch.setattr(
        chat, "stranger_response_body", lambda raw, command: (1, "failed", None, None)
    )

    with pytest.raises(DoudouError, match="Stranger conversation protocol"):
        asyncio.run(chat.refresh_session_index(session, tmp_path, "work", force=True))

    stored = json.loads(
        (tmp_path / "message" / "profiles" / "work" / "conversations.json").read_text()
    )
    assert stored == previous


def test_refresh_rejects_stalled_cursor_and_preserves_index(tmp_path, monkeypatch):
    previous = write_conversation_index(tmp_path, "work", [established()])
    session = Session()

    async def post_protobuf(endpoint, request):
        return 200, b"response"

    session.post_protobuf = post_protobuf
    monkeypatch.setattr(chat, "find_conversations", lambda raw: [established()])
    monkeypatch.setattr(chat, "stranger_request", lambda *args: b"request")
    monkeypatch.setattr(
        chat, "stranger_response_body", lambda raw, command: (0, "OK", b"payload", None)
    )
    monkeypatch.setattr(
        chat,
        "parse_stranger_conversations",
        lambda payload: {
            "conversations": [stranger()],
            "next_cursor": "0",
            "has_more": True,
            "total_unread": 1,
        },
    )

    async def no_enrichment(*args):
        return None

    monkeypatch.setattr(chat, "_enrich_conversation_identities", no_enrichment)
    with pytest.raises(DoudouError, match="cursor did not advance"):
        asyncio.run(chat.refresh_session_index(session, tmp_path, "work", force=True))

    stored = json.loads(
        (tmp_path / "message" / "profiles" / "work" / "conversations.json").read_text()
    )
    assert stored == previous


def test_refresh_merges_established_and_request_by_default(tmp_path, monkeypatch):
    session = Session()

    async def post_protobuf(endpoint, request):
        return 200, b"response"

    session.post_protobuf = post_protobuf
    monkeypatch.setattr(chat, "find_conversations", lambda raw: [established()])
    monkeypatch.setattr(chat, "stranger_request", lambda *args: b"request")
    monkeypatch.setattr(
        chat, "stranger_response_body", lambda raw, command: (0, "OK", b"payload", None)
    )
    monkeypatch.setattr(
        chat,
        "parse_stranger_conversations",
        lambda payload: {
            "conversations": [stranger()],
            "next_cursor": "0",
            "has_more": False,
            "total_unread": 1,
        },
    )

    async def no_enrichment(*args):
        return None

    monkeypatch.setattr(chat, "_enrich_conversation_identities", no_enrichment)
    result = asyncio.run(
        chat.refresh_session_index(session, tmp_path, "work", force=True)
    )

    values = result["index"]["conversations"]
    assert [item["conversation_id"] for item in values] == ["known", "request"]
    assert values[1]["inbox"] == "request"
    assert result["snapshot"]["inbox_count"] == 1


def test_conversations_filters_inbox_without_changing_index(tmp_path, monkeypatch):
    async def refreshed(*args, **kwargs):
        return {
            "source": "live",
            "index": {
                "refreshed_at": "2026-01-01T00:00:00Z",
                "conversations": [established(), stranger()],
            },
        }

    monkeypatch.setattr(service, "refresh_session_index", refreshed)
    result = asyncio.run(
        service.conversations(Session(), tmp_path, "work", force=True, inbox="request")
    )
    assert result["count"] == 1
    assert result["conversations"][0]["conversation_id"] == "request"


def test_collect_routes_request_and_enriches_sender(tmp_path, monkeypatch):
    session = Session()
    calls = []

    async def post_protobuf(endpoint, request):
        calls.append((endpoint, request))
        return 200, b"response"

    session.post_protobuf = post_protobuf

    async def refreshed(*args, **kwargs):
        return {"index": {"conversations": [stranger()]}}

    monkeypatch.setattr(chat, "refresh_session_index", refreshed)
    monkeypatch.setattr(chat, "stranger_messages_request", lambda short, reset_unread_count: (short, reset_unread_count))
    monkeypatch.setattr(chat, "stranger_request", lambda init, command, body: (command, body))
    monkeypatch.setattr(chat, "stranger_response_body", lambda raw, command: (0, "OK", b"", None))
    monkeypatch.setattr(
        chat,
        "parse_stranger_messages",
        lambda payload, conversation_id: [
            {
                "message_id": "1",
                "conversation_id": conversation_id,
                "create_time_ms": "1",
                "message_type": 7,
                "sender": {"sec_uid": "sec-request"},
                "unknown_fields": [],
            }
        ],
    )

    async def identities(*args, **kwargs):
        return {"sec-request": {"sec_uid": "sec-request", "display_name": "Nickname"}}

    monkeypatch.setattr(chat, "_resolve_user_identities", identities)
    result = asyncio.run(
        chat.collect_messages(session, tmp_path, "work", "request", download_images=False)
    )

    assert calls[0][0] == chat.STRANGER_MESSAGES_ENDPOINT
    assert calls[0][1] == (1002, ("20", False))
    assert result["message_count"] == 1
    assert result["pages"][0]["protocol_received"] == 1
    assert result["pages"][0]["summary_received"] == 0
    assert result["pages"][0]["collection_source"] == "protocol"
    assert result["conversation"]["inbox"] == "request"
    assert result["conversation"]["last_message"]["sender"]["display_name"] == "Nickname"
    messages = json.loads(
        (tmp_path / result["merge"]["messages_file"]).read_text().splitlines()[0]
    )
    assert messages["sender"]["name"] == "Nickname"


def test_request_display_name_uses_counterparty_nickname(tmp_path, monkeypatch):
    session = Session()
    request = stranger()
    request["participants"] = {
        "participants": [
            {"uid": "self", "sec_uid": "sec-self", "display_name": "My Account"},
            {"uid": "other", "sec_uid": "sec-request", "display_name": None},
        ],
        "has_more": False,
        "cursor": "0",
    }
    known = established()
    known["participants"] = {
        "participants": [
            {"uid": "self", "sec_uid": "sec-self", "display_name": "My Account"},
            {"uid": "known-other", "sec_uid": "sec-known", "display_name": "Known"},
        ],
        "has_more": False,
        "cursor": "0",
    }

    async def resolve(*args, **kwargs):
        return {
            "sec-self": {"uid": "self", "sec_uid": "sec-self", "display_name": "My Account"},
            "sec-request": {"uid": "other", "sec_uid": "sec-request", "display_name": "Counterparty Nickname"},
        }

    monkeypatch.setattr(chat, "_resolve_user_identities", resolve)
    asyncio.run(
        chat._enrich_conversation_identities(
            session, tmp_path, "work", [known, request]
        )
    )

    assert request["display_name"] == "Counterparty Nickname"
    assert request["display_name"] != "My Account"
    assert request["display_name"] != request["conversation_id"]


def test_request_display_name_does_not_use_self_sender_when_self_uid_is_unknown(
    tmp_path, monkeypatch
):
    session = Session()
    request = stranger()
    request["participants"] = {
        "participants": [
            {"uid": "self", "sec_uid": "sec-self", "display_name": None},
            {"uid": "other", "sec_uid": "sec-request", "display_name": None},
        ],
        "has_more": False,
        "cursor": "0",
    }
    request["last_message"]["sender"] = {"sec_uid": "sec-self"}

    async def resolve(*args, **kwargs):
        return {
            "sec-self": {
                "uid": "self",
                "sec_uid": "sec-self",
                "display_name": "My Account",
            },
            "sec-request": {
                "uid": "other",
                "sec_uid": "sec-request",
                "display_name": "Counterparty Nickname",
            },
        }

    monkeypatch.setattr(chat, "_resolve_user_identities", resolve)
    asyncio.run(
        chat._enrich_conversation_identities(session, tmp_path, "work", [request])
    )

    assert request["display_name"] == "Request"
    assert request["display_name"] != "My Account"


def test_collect_empty_1002_payload_uses_visible_conversation_summary(
    tmp_path, monkeypatch
):
    session = Session()
    request = stranger()
    request["last_message"]["message_id"] = "2"
    request["last_message"]["text"] = "Visible request message"

    async def refreshed(*args, **kwargs):
        return {"index": {"conversations": [request]}}

    async def post_protobuf(endpoint, request):
        return 200, b"response"

    session.post_protobuf = post_protobuf
    monkeypatch.setattr(chat, "refresh_session_index", refreshed)
    monkeypatch.setattr(
        chat, "stranger_messages_request", lambda short, reset_unread_count: (short, reset_unread_count)
    )
    monkeypatch.setattr(chat, "stranger_request", lambda init, command, body: (command, body))
    monkeypatch.setattr(chat, "stranger_response_body", lambda raw, command: (0, "OK", b"", None))
    monkeypatch.setattr(chat, "parse_stranger_messages", lambda payload, conversation_id: [])
    async def no_identities(*args, **kwargs):
        return {}

    monkeypatch.setattr(chat, "_resolve_user_identities", no_identities)

    result = asyncio.run(
        chat.collect_messages(session, tmp_path, "work", "request", download_images=False)
    )

    assert result["message_count"] == 1
    assert result["pages"][0]["protocol_received"] == 0
    assert result["pages"][0]["summary_received"] == 1
    assert result["pages"][0]["collection_source"] == "conversation_summary"
    assert result["pages"][0]["fallback_used"] is False
    assert result["pages"][0]["protocol_http_status"] == 200
    assert result["pages"][0]["protocol_status"] == 0
    assert result["pages"][0]["payload_bytes"] == 0
    stored = json.loads(
        (tmp_path / result["merge"]["messages_file"]).read_text().splitlines()[0]
    )
    assert stored["message_id"] == "2"
    assert stored["collection_source"] == "stranger-conversation-summary"


def test_collect_empty_1002_without_summary_stays_empty(tmp_path, monkeypatch):
    session = Session()
    request = stranger()
    request["last_message"] = None

    async def refreshed(*args, **kwargs):
        return {"index": {"conversations": [request]}}

    async def post_protobuf(endpoint, request):
        return 200, b"response"

    session.post_protobuf = post_protobuf
    monkeypatch.setattr(chat, "refresh_session_index", refreshed)
    monkeypatch.setattr(
        chat,
        "stranger_messages_request",
        lambda short, reset_unread_count: (short, reset_unread_count),
    )
    monkeypatch.setattr(
        chat, "stranger_request", lambda init, command, body: (command, body)
    )
    monkeypatch.setattr(
        chat,
        "stranger_response_body",
        lambda raw, command: (0, "OK", b"", None),
    )
    monkeypatch.setattr(
        chat, "parse_stranger_messages", lambda payload, conversation_id: []
    )

    async def no_identities(*args, **kwargs):
        return {}

    monkeypatch.setattr(chat, "_resolve_user_identities", no_identities)
    result = asyncio.run(
        chat.collect_messages(
            session, tmp_path, "work", "request", download_images=False
        )
    )

    assert result["message_count"] == 0
    assert result["pages"][0]["protocol_received"] == 0
    assert result["pages"][0]["summary_received"] == 0
    assert result["pages"][0]["collection_source"] == "empty"


def test_1002_request_uses_body_field_1001_and_does_not_reset_unread():
    body = chat.stranger_messages_request("20", reset_unread_count=False)
    envelope = chat.stranger_request(b"", 1002, body, sequence=1)
    outer = decode_fields(envelope)
    command_body = get_field(decode_fields(bytes(get_field(outer, 8, 2).value)), 1001, 2)
    assert command_body is not None
    assert decode_fields(bytes(command_body.value)) == decode_fields(body)
    assert get_field(decode_fields(body), 2, 0).value == 0


@pytest.mark.parametrize("operation", ["send", "batch", "like"])
def test_write_previews_reject_request_before_plan(tmp_path, monkeypatch, operation):
    async def refreshed(*args, **kwargs):
        return {"index": {"conversations": [stranger()]}}

    monkeypatch.setattr(service, "refresh_session_index", refreshed)
    monkeypatch.setattr(
        service,
        "create_plan",
        lambda *args, **kwargs: pytest.fail("plan must not be created"),
    )
    if operation == "send":
        call = service.preview_send(Session(), tmp_path, "work", {"target": "request", "text": "x"})
    elif operation == "batch":
        call = service.preview_batch(Session(), tmp_path, "work", {"targets": ["request"], "text": "x"})
    else:
        call = service.preview_like(Session(), tmp_path, "work", {"target": "request", "message": "1"})
    with pytest.raises(DoudouError, match="read-only"):
        asyncio.run(call)
