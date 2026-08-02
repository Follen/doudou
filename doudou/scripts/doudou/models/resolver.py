from __future__ import annotations

from datetime import datetime
from typing import Any

from ..common.core import normalize_lookup
from ..common.errors import DoudouError


def resolve_conversation(
    conversations: list[dict[str, Any]], target: str
) -> dict[str, Any]:
    raw = str(target or "").strip()
    if not raw:
        raise DoudouError("MISSING_ARGUMENT", "Missing a conversation id or name.")
    for item in conversations:
        if str(item.get("conversation_id")) == raw:
            return item
    normalized = normalize_lookup(raw)
    matches = [
        item
        for item in conversations
        if normalized in normalize_lookup(item.get("display_name"))
    ]
    if not matches:
        raise DoudouError(
            "RECIPIENT_NOT_FOUND",
            f"No existing conversation matches: {raw}.",
            {"target": raw, "candidates": []},
        )
    if len(matches) > 1:
        raise DoudouError(
            "CONVERSATION_AMBIGUOUS",
            f"More than one conversation matches: {raw}.",
            {
                "target": raw,
                "candidates": [
                    {"number": conversations.index(item) + 1, **item}
                    for item in matches
                ],
            },
        )
    return matches[0]


def resolve_message(messages: list[dict[str, Any]], target: str) -> dict[str, Any]:
    raw = str(target or "").strip()
    if not raw:
        raise DoudouError("MISSING_ARGUMENT", "Missing a message id or text.")
    for item in messages:
        if str(item.get("message_id")) == raw:
            return item
    normalized = normalize_lookup(raw)
    matches = [
        item for item in messages if normalized in normalize_lookup(item.get("text"))
    ]
    if not matches:
        raise DoudouError(
            "MESSAGE_NOT_FOUND",
            f"No collected message contains: {raw}.",
            {"query": raw, "candidates": []},
        )
    if len(matches) > 1:
        raise DoudouError(
            "MESSAGE_AMBIGUOUS",
            f"More than one message contains: {raw}.",
            {"query": raw, "candidates": matches},
        )
    return matches[0]


def parse_selection(value: str, count: int) -> list[int]:
    text = str(value or "").strip()
    if text in {"全部", "all", "ALL"}:
        return list(range(1, count + 1))
    selected: set[int] = set()
    for token in text.split(","):
        token = token.strip()
        if not token:
            continue
        if "-" in token:
            left, right = token.split("-", 1)
            start, end = int(left), int(right)
            if start > end:
                raise DoudouError(
                    "INVALID_CONVERSATION_RANGE",
                    "Selection range starts after it ends.",
                )
            selected.update(range(start, end + 1))
        else:
            selected.add(int(token))
    if not selected or min(selected) < 1 or max(selected) > count:
        raise DoudouError(
            "CONVERSATION_NUMBER_OUT_OF_RANGE",
            "Selection contains an out-of-range conversation number.",
        )
    return sorted(selected)


def default_message_window(clock: datetime) -> tuple[str, str]:
    local_clock = clock if clock.utcoffset() is not None else clock.astimezone()
    start = local_clock.replace(hour=0, minute=0, second=0, microsecond=0)
    return start.isoformat(), local_clock.isoformat()


def infer_self_uid(conversations: list[dict[str, Any]]) -> str | None:
    counts: dict[str, int] = {}
    for conversation in conversations:
        if int(conversation.get("conversation_type") or 0) != 1:
            continue
        participants = (conversation.get("participants") or {}).get(
            "participants", []
        )
        for participant in participants:
            uid = str(participant.get("uid") or "")
            if uid:
                counts[uid] = counts.get(uid, 0) + 1
    ranked = sorted(counts.items(), key=lambda item: (-item[1], item[0]))
    if not ranked or (len(ranked) > 1 and ranked[0][1] == ranked[1][1]):
        return None
    return ranked[0][0]
