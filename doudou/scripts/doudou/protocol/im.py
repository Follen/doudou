from __future__ import annotations

import json
import time
from typing import Any

from ..models.identity import normalize_identity
from ..models.records import conversation_kind
from .protobuf import (
    Field,
    decode_fields,
    encode_fields,
    get_field,
    get_fields,
    replace_field,
    string_field,
    text_value,
    varint_field,
)

# Stranger inbox uses a different body field than its command id.
STRANGER_COMMAND_BODY_FIELDS = {1001: 1000, 1002: 1001}


def stranger_body_field(command: int) -> int:
    """Return the protobuf body field used by a stranger-inbox command."""
    try:
        return STRANGER_COMMAND_BODY_FIELDS[command]
    except KeyError as exc:
        raise ValueError(f"unsupported stranger command: {command}") from exc


def stranger_request(
    init_request: bytes, command: int, body: bytes, sequence: int | None = None
) -> bytes:
    """Build a command envelope using the stranger command/body mapping."""
    outer = decode_fields(init_request)
    outer = replace_field(outer, 1, varint_field(1, command))
    outer = replace_field(
        outer, 2, varint_field(2, sequence or int(time.time() * 1_000_000))
    )
    outer = replace_field(
        outer,
        8,
        Field(8, 2, encode_fields([Field(stranger_body_field(command), 2, body)])),
    )
    return encode_fields(outer)


def stranger_conversations_request(
    cursor: int | str = 0, count: int = 50, show_total_unread: bool = True
) -> bytes:
    return encode_fields(
        [
            varint_field(1, int(cursor)),
            varint_field(2, max(1, int(count))),
            varint_field(3, int(bool(show_total_unread))),
        ]
    )


def stranger_messages_request(
    conversation_short_id: str | int, reset_unread_count: bool = False
) -> bytes:
    return encode_fields(
        [
            varint_field(1, int(conversation_short_id)),
            varint_field(2, int(bool(reset_unread_count))),
        ]
    )


def stranger_response_body(
    data: bytes, command: int
) -> tuple[int, str, bytes | None, dict[str, Any] | None]:
    """Parse a stranger response using its mapped body field."""
    return response_body(data, stranger_body_field(command))


def _stranger_record(fields: list[Field]) -> dict[str, Any] | None:
    short = get_field(fields, 1, 0)
    identity = text_value(get_field(fields, 4, 2))
    if not short or not identity:
        return None
    participants: list[dict[str, Any]] = []
    for item in get_fields(fields, 5, 2):
        page = parse_participants_page(
            encode_fields([Field(1, 2, bytes(item.value))])
        )
        participants.extend(page["participants"])
    last = get_field(fields, 3, 2)
    last_message = (
        normalize_message(bytes(last.value), identity) if last is not None else None
    )
    unread = get_field(fields, 2, 0)
    names = {
        str(item.get("display_name") or item.get("alias") or "").strip()
        for item in participants
    }
    names.discard("")
    return {
        "conversation_id": identity,
        "conversation_short_id": str(short.value),
        "conversation_type": 1,
        "kind": "direct",
        "inbox": "request",
        "display_name": next(iter(names)) if len(names) == 1 else identity,
        "unread_count": int(unread.value) if unread else 0,
        "participants": {
            "participants": participants,
            "has_more": False,
            "cursor": "0",
        },
        "last_message": last_message,
        "ext": parse_map_entries(fields, 7),
    }


def parse_stranger_conversations(body: bytes) -> dict[str, Any]:
    """Normalize 1001 response body and its nested conversation records."""
    try:
        fields = decode_fields(body)
    except ValueError:
        return {
            "conversations": [],
            "next_cursor": "0",
            "has_more": False,
            "total_unread": 0,
        }
    conversations: list[dict[str, Any]] = []
    for item in get_fields(fields, 4, 2):
        try:
            record = _stranger_record(decode_fields(bytes(item.value)))
        except ValueError:
            continue
        if record:
            conversations.append(record)
    return {
        "conversations": conversations,
        "next_cursor": str(get_field(fields, 1, 0).value)
        if get_field(fields, 1, 0)
        else "0",
        "has_more": bool(get_field(fields, 2, 0).value)
        if get_field(fields, 2, 0)
        else False,
        "total_unread": int(get_field(fields, 3, 0).value)
        if get_field(fields, 3, 0)
        else 0,
    }


def parse_stranger_messages(
    body: bytes, conversation_id: str | None = None
) -> list[dict[str, Any]]:
    """Extract standard message records from a 1002 response."""
    try:
        fields = decode_fields(body)
    except ValueError:
        return []
    output: list[dict[str, Any]] = []
    for item in (
        get_fields(fields, 3, 2) + get_fields(fields, 1, 2) + get_fields(fields, 2, 2)
    ):
        value = bytes(item.value)
        message = normalize_message(value, conversation_id) if conversation_id else None
        if message:
            output.append(message)
        else:
            output.extend(parse_stranger_messages(value, conversation_id))
    return output


def parse_map_entries(fields: list[Field], number: int) -> dict[str, str]:
    output: dict[str, str] = {}
    for item in get_fields(fields, number, 2):
        try:
            entry = decode_fields(bytes(item.value))
        except ValueError:
            continue
        key, value = (
            text_value(get_field(entry, 1, 2)),
            text_value(get_field(entry, 2, 2)),
        )
        if key and value is not None:
            output[key] = value
    return output


def map_entry(key: str, value: str) -> Field:
    return Field(5, 2, encode_fields([string_field(1, key), string_field(2, value)]))


def structured_field(item: Field) -> dict[str, Any]:
    if item.wire == 0:
        return {"field": item.number, "wire": 0, "value": str(item.value)}
    value = bytes(item.value)
    text = text_value(item)
    return {
        "field": item.number,
        "wire": item.wire,
        **(
            {"text": text[:500]}
            if text is not None
            else {"prefix_hex": value[:16].hex()}
        ),
        "bytes": len(value),
    }


def build_command_request(
    init_request: bytes, command: int, body: bytes, sequence: int | None = None
) -> bytes:
    outer = decode_fields(init_request)
    outer = replace_field(outer, 1, varint_field(1, command))
    outer = replace_field(
        outer, 2, varint_field(2, sequence or int(time.time() * 1_000_000))
    )
    outer = replace_field(
        outer, 8, Field(8, 2, encode_fields([Field(command, 2, body)]))
    )
    return encode_fields(outer)


def response_body(
    data: bytes, command: int
) -> tuple[int, str, bytes | None, dict[str, Any] | None]:
    if data.lstrip().startswith((b"{", b"[")):
        try:
            gateway = json.loads(data.decode("utf-8"))
        except (UnicodeDecodeError, json.JSONDecodeError):
            gateway = {"invalid_json": True}
        return -1, "json gateway", None, gateway
    outer = decode_fields(data)
    status_field = get_field(outer, 3, 0)
    payload = get_field(outer, 6, 2)
    if not payload:
        return (
            int(status_field.value) if status_field else -1,
            text_value(get_field(outer, 4, 2)) or "",
            None,
            None,
        )
    wrapper = decode_fields(bytes(payload.value))
    body = get_field(wrapper, command, 2)
    return (
        int(status_field.value) if status_field else -1,
        text_value(get_field(outer, 4, 2)) or "",
        bytes(body.value) if body else None,
        None,
    )


def find_conversations(data: bytes) -> list[dict[str, Any]]:
    found: dict[str, dict[str, Any]] = {}
    pending: list[tuple[bytes, int]] = [(data, 0)]
    while pending:
        value, depth = pending.pop()
        if depth > 10 or not value:
            continue
        try:
            fields = decode_fields(value)
        except ValueError:
            continue
        identity = text_value(get_field(fields, 1, 2))
        short, kind, ticket = (
            get_field(fields, 2, 0),
            get_field(fields, 3, 0),
            text_value(get_field(fields, 4, 2)),
        )
        if identity and short and kind and ticket and int(kind.value) in (1, 2):
            core = get_field(fields, 50, 2)
            display_name = None
            if core:
                try:
                    core_fields = decode_fields(bytes(core.value))
                    core_identity = text_value(get_field(core_fields, 1, 2))
                    if core_identity == identity:
                        display_name = text_value(get_field(core_fields, 5, 2))
                except ValueError:
                    pass
            found[identity] = {
                "conversation_id": identity,
                "conversation_short_id": str(short.value),
                "conversation_type": int(kind.value),
                "kind": conversation_kind(int(kind.value)),
                "inbox": "established",
                "display_name": display_name or identity,
                "participants": parse_participants_page(
                    bytes(get_field(fields, 6, 2).value)
                    if get_field(fields, 6, 2)
                    else b""
                ),
            }
        pending.extend(
            (bytes(item.value), depth + 1)
            for item in fields
            if item.wire == 2 and len(bytes(item.value)) <= 10 * 1024 * 1024
        )
    return list(found.values())


def parse_participants_page(data: bytes) -> dict[str, Any]:
    if not data:
        return {"participants": [], "has_more": False, "cursor": "0"}
    try:
        fields = decode_fields(data)
    except ValueError:
        return {"participants": [], "has_more": False, "cursor": "0"}
    participants: list[dict[str, Any]] = []
    for item in get_fields(fields, 1, 2):
        try:
            record = decode_fields(bytes(item.value))
        except ValueError:
            continue
        uid = get_field(record, 1, 0)
        if not uid:
            continue
        alias = text_value(get_field(record, 4, 2)) or ""
        participants.append(
            {
                **normalize_identity(
                    {
                        "uid": str(uid.value),
                        "sec_uid": text_value(get_field(record, 5, 2)) or "",
                        "remark_name": alias,
                    },
                    source="im-participant",
                ),
                "sort_order": str(get_field(record, 2, 0).value)
                if get_field(record, 2, 0)
                else None,
                "role": int(get_field(record, 3, 0).value)
                if get_field(record, 3, 0)
                else 0,
                "alias": alias,
                "ext": parse_map_entries(record, 8),
            }
        )
    return {
        "participants": participants,
        "has_more": bool(int(get_field(fields, 2, 0).value))
        if get_field(fields, 2, 0)
        else False,
        "cursor": str(get_field(fields, 3, 0).value)
        if get_field(fields, 3, 0)
        else "0",
    }


def participants_request(
    conversation: dict[str, Any], cursor: int | str = 0, limit: int = 100
) -> bytes:
    return encode_fields(
        [
            string_field(1, str(conversation["conversation_id"])),
            varint_field(2, int(conversation["conversation_short_id"])),
            varint_field(3, int(conversation["conversation_type"])),
            varint_field(4, int(cursor)),
            varint_field(5, limit),
        ]
    )


def normalize_system_event(content: Any, message_type: int) -> dict[str, Any] | None:
    if message_type not in (1001, 50001):
        return None
    if not isinstance(content, dict) or content.get("aweType") != 100140:
        return {
            "kind": "unknown",
            "protocol": {
                "message_type": message_type,
                "awe_type": content.get("aweType")
                if isinstance(content, dict)
                else None,
            },
            "content_shape": sorted(content)
            if isinstance(content, dict)
            else type(content).__name__,
        }

    def users(key: str) -> list[dict[str, Any]]:
        return [
            normalize_identity(item, source=f"member-joined-{key}")
            for item in content.get(key) or []
        ]

    return {
        "kind": "member_joined",
        "protocol": {"message_type": 1001, "awe_type": 100140},
        "join_method": "profile_homepage",
        "history_visible": True,
        "source_users": users("active_users"),
        "joined_users": users("passive_users"),
        "locale_resources": content.get("locale_resources") or [],
        "template_actions": content.get("template") or [],
        "content_ext": content.get("content_ext"),
    }


def parse_reference_info(item: Field | None) -> dict[str, Any] | None:
    if not item or item.wire != 2:
        return None
    try:
        fields = decode_fields(bytes(item.value))
    except ValueError:
        return None
    raw = text_value(get_field(fields, 2, 2))
    try:
        hint: Any = json.loads(raw) if raw is not None else None
    except (TypeError, json.JSONDecodeError):
        hint = raw
    known = {1, 2, 3, 4, 5, 6, 7}
    return {
        "referenced_message_id": str(get_field(fields, 1, 0).value)
        if get_field(fields, 1, 0)
        else None,
        "hint": hint,
        "hint_raw": raw,
        "ref_message_type": int(get_field(fields, 3, 0).value)
        if get_field(fields, 3, 0)
        else None,
        "referenced_message_status": int(get_field(fields, 4, 0).value)
        if get_field(fields, 4, 0)
        else None,
        "root_message_id": str(get_field(fields, 5, 0).value)
        if get_field(fields, 5, 0)
        else None,
        "root_message_conv_index": str(get_field(fields, 6, 0).value)
        if get_field(fields, 6, 0)
        else None,
        "ext": parse_map_entries(fields, 7),
        "unknown_fields": [
            structured_field(field) for field in fields if field.number not in known
        ],
    }


def parse_property_list(fields: list[Field]) -> dict[str, Any]:
    output: dict[str, Any] = {}
    for item in get_fields(fields, 15, 2):
        try:
            entry = decode_fields(bytes(item.value))
            key, value = text_value(get_field(entry, 1, 2)), get_field(entry, 2, 2)
            if not key or not value:
                continue
            list_fields = decode_fields(bytes(value.value))
            items: list[dict[str, Any]] = []
            for child in get_fields(list_fields, 1, 2):
                fields2 = decode_fields(bytes(child.value))
                identity = normalize_identity(
                    {
                        "uid": str(get_field(fields2, 1, 0).value)
                        if get_field(fields2, 1, 0)
                        else None,
                        "sec_uid": text_value(get_field(fields2, 2, 2)),
                    },
                    source=f"message-property-{key}",
                )
                items.append(
                    {
                        **identity,
                        "create_time": str(get_field(fields2, 3, 0).value)
                        if get_field(fields2, 3, 0)
                        else None,
                        "idempotent_id": text_value(get_field(fields2, 4, 2)),
                        "value": text_value(get_field(fields2, 5, 2)),
                    }
                )
            output[key] = {"items": items}
        except ValueError:
            continue
    return output


def property_has_user(property_list: dict[str, Any] | None, uid: str) -> bool:
    return any(
        str(item.get("uid") or "") == str(uid)
        for value in (property_list or {}).values()
        if isinstance(value, dict)
        for item in value.get("items") or []
        if isinstance(item, dict)
    )


def normalize_message(data: bytes, expected_conversation: str) -> dict[str, Any] | None:
    try:
        fields = decode_fields(data)
    except ValueError:
        return None
    conversation_id = text_value(get_field(fields, 1, 2))
    message_id, message_type, create_time = (
        get_field(fields, 3, 0),
        get_field(fields, 6, 0),
        get_field(fields, 10, 0),
    )
    content_text = text_value(get_field(fields, 8, 2))
    if (
        conversation_id != expected_conversation
        or not message_id
        or not message_type
        or not create_time
        or content_text is None
    ):
        return None
    try:
        content: Any = json.loads(content_text)
    except (TypeError, json.JSONDecodeError):
        content = content_text
    message_type_value = int(message_type.value)
    reference = parse_reference_info(get_field(fields, 18, 2))
    kind = (
        "system"
        if message_type_value in (1001, 50001)
        else "image"
        if message_type_value == 27
        else "text"
        if message_type_value == 7
        else "unsupported"
    )
    known = set(range(1, 16)) | {17, 18}
    return {
        "schema": "doudou.message.v2",
        "message_id": str(message_id.value),
        "conversation_id": conversation_id,
        "conversation_type": int(get_field(fields, 2, 0).value)
        if get_field(fields, 2, 0)
        else None,
        "conversation_short_id": str(get_field(fields, 5, 0).value)
        if get_field(fields, 5, 0)
        else None,
        "message_type": message_type_value,
        "type": kind,
        "sender": normalize_identity(
            {
                "uid": str(get_field(fields, 7, 0).value)
                if get_field(fields, 7, 0)
                else None,
                "sec_uid": text_value(get_field(fields, 14, 2)),
            },
            source="im-message",
        ),
        "create_time_ms": int(create_time.value),
        "text": content.get("text")
        if isinstance(content, dict)
        else content
        if isinstance(content, str) and kind == "text"
        else None,
        "content": content,
        "system_event": normalize_system_event(content, message_type_value),
        "reference_info": reference,
        "reply_to_message_id": reference.get("referenced_message_id")
        if reference
        else None,
        "ext": parse_map_entries(fields, 9),
        "property_list": parse_property_list(fields),
        "unknown_fields": [
            structured_field(item) for item in fields if item.number not in known
        ],
        "images": [],
    }


def extract_messages(body: bytes, conversation_id: str) -> list[dict[str, Any]]:
    output: dict[str, dict[str, Any]] = {}
    pending: list[tuple[bytes, int]] = [(body, 0)]
    while pending:
        value, depth = pending.pop()
        if depth > 8 or not value:
            continue
        message = normalize_message(value, conversation_id)
        if message:
            output[message["message_id"]] = message
        try:
            fields = decode_fields(value)
        except ValueError:
            continue
        pending.extend(
            (bytes(item.value), depth + 1)
            for item in fields
            if item.wire == 2 and len(bytes(item.value)) <= 10 * 1024 * 1024
        )
    return sorted(
        output.values(),
        key=lambda item: (item["create_time_ms"], int(item["message_id"])),
    )


def messages_request(
    conversation: dict[str, Any],
    anchor_index: int = 0,
    limit: int = 50,
    direction: int = 1,
) -> bytes:
    return encode_fields(
        [
            string_field(1, str(conversation["conversation_id"])),
            varint_field(2, int(conversation["conversation_type"])),
            varint_field(3, int(conversation["conversation_short_id"])),
            varint_field(4, direction),
            varint_field(5, anchor_index),
            varint_field(6, limit),
        ]
    )


def parse_send_request(data: bytes) -> dict[str, Any]:
    outer = decode_fields(data)
    command_wrapper = get_field(outer, 8, 2)
    if not command_wrapper:
        raise ValueError("send request command wrapper is missing")
    command_fields = decode_fields(bytes(command_wrapper.value))
    body_field = get_field(command_fields, 100, 2)
    if not body_field:
        raise ValueError("send request body is missing")
    fields = decode_fields(bytes(body_field.value))
    content_text = text_value(get_field(fields, 4, 2))
    try:
        content: Any = json.loads(content_text) if content_text is not None else None
    except json.JSONDecodeError:
        content = content_text
    reference = get_field(fields, 11, 2)
    reference_id = None
    if reference:
        reference_fields = decode_fields(bytes(reference.value))
        reference_value = get_field(reference_fields, 1, 0)
        reference_id = str(reference_value.value) if reference_value else None
    return {
        "conversation_id": text_value(get_field(fields, 1, 2)),
        "conversation_type": int(get_field(fields, 2, 0).value)
        if get_field(fields, 2, 0)
        else None,
        "conversation_short_id": str(get_field(fields, 3, 0).value)
        if get_field(fields, 3, 0)
        else None,
        "content": content,
        "ext": parse_map_entries(fields, 5),
        "message_type": int(get_field(fields, 6, 0).value)
        if get_field(fields, 6, 0)
        else None,
        "client_message_id": text_value(get_field(fields, 8, 2)),
        "mentioned_users": [str(item.value) for item in get_fields(fields, 9, 0)],
        "referenced_message_id": reference_id,
    }


def parse_send_response(data: bytes) -> dict[str, Any]:
    status, status_text, body, gateway = response_body(data, 100)
    if gateway is not None:
        return {
            "protobuf_status": status,
            "status_text": status_text,
            "gateway": gateway,
        }
    body_fields = decode_fields(body) if body else []
    return {
        "protobuf_status": status,
        "status_text": status_text,
        "gateway": None,
        "server_message_id": str(get_field(body_fields, 1, 0).value)
        if get_field(body_fields, 1, 0)
        else None,
        "business_status": int(get_field(body_fields, 3, 0).value)
        if get_field(body_fields, 3, 0)
        else None,
        "client_message_id": text_value(get_field(body_fields, 4, 2)),
        "check_code": int(get_field(body_fields, 5, 0).value)
        if get_field(body_fields, 5, 0)
        else None,
    }
