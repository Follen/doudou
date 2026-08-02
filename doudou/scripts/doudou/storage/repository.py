from __future__ import annotations

from pathlib import Path
from typing import Any

from ..common.core import (
    atomic_write_json,
    atomic_write_jsonl,
    read_json,
    read_jsonl,
    safe_segment,
    sha256,
    utc_now,
)
from ..models.identity import identity_matches, merge_identity
from ..runtime.paths import profile_paths

IDENTITY_LIST_FIELDS = {
    "mentioned_users",
    "system_event.joined_users",
    "system_event.source_users",
}

AUTHORITATIVE_INCOMING_FIELDS = {
    "schema",
    "type",
    "text",
    "content",
    "system_event",
    "property_list",
    "reference_info",
    "unknown_fields",
}


def conversation_folder(conversation: dict[str, Any]) -> str:
    return f"{conversation.get('kind', 'unknown')}__{safe_segment(conversation.get('display_name') or 'unnamed', 64)}__{safe_segment(conversation['conversation_id'], 64)}"


def write_conversation_index(
    data_root: Path, profile: str, conversations: list[dict[str, Any]]
) -> dict[str, Any]:
    paths = profile_paths(data_root, profile)
    paths.message.mkdir(parents=True, exist_ok=True)
    refreshed_at = utc_now()
    normalized = []
    for item in conversations:
        value = {
            **item,
            "folder_name": conversation_folder(item),
            "updated_at": refreshed_at,
        }
        normalized.append(value)
    index = {
        "schema": "doudou.conversations-index.v2",
        "profile_id": paths.profile_id,
        "refreshed_at": refreshed_at,
        "count": len(normalized),
        "conversations": normalized,
    }
    atomic_write_json(paths.message / "conversations.json", index)
    return index


def read_conversation_index(data_root: Path, profile: str) -> dict[str, Any] | None:
    return read_json(
        profile_paths(data_root, profile).message / "conversations.json", optional=True
    )


def write_session_index_snapshot(
    data_root: Path, profile: str, snapshot: dict[str, Any]
) -> dict[str, str]:
    paths = profile_paths(data_root, profile)
    completed = snapshot["completed_at"]
    day = completed[:10]
    name = (
        completed.replace("-", "").replace(":", "").replace(".", "")[:18]
        + "__session-index.json"
    )
    filename = paths.session_index / day / name
    atomic_write_json(filename, snapshot)
    atomic_write_json(paths.session_index / "latest.json", snapshot)
    return {
        "snapshot": str(filename.relative_to(data_root)),
        "latest": str((paths.session_index / "latest.json").relative_to(data_root)),
    }


def read_session_index_snapshot(data_root: Path, profile: str) -> dict[str, Any] | None:
    return read_json(
        profile_paths(data_root, profile).session_index / "latest.json", optional=True
    )


def merge_messages(
    data_root: Path,
    profile: str,
    conversation: dict[str, Any],
    incoming: list[dict[str, Any]],
) -> dict[str, Any]:
    paths = profile_paths(data_root, profile)
    directory = paths.conversations / conversation_folder(conversation)
    directory.mkdir(parents=True, exist_ok=True)
    metadata = {
        "schema": "doudou.conversation.v2",
        **conversation,
        "profile_id": paths.profile_id,
        "folder_name": directory.name,
        "updated_at": utc_now(),
    }
    atomic_write_json(directory / "conversation.json", metadata)
    existing = read_jsonl(directory / "messages.jsonl", optional=True)
    by_id = {str(item["message_id"]): item for item in existing}
    conflicts: list[dict[str, Any]] = read_jsonl(
        directory / "conflicts.jsonl", optional=True
    )
    existing_conflict_keys = {
        (
            str(item.get("message_id") or ""),
            str(item.get("field") or ""),
            str(item.get("existing_sha256") or ""),
            str(item.get("incoming_sha256") or ""),
        )
        for item in conflicts
    }
    new_conflicts = 0

    def record_conflict(
        message_id: str, field_path: str, existing: Any, candidate: Any
    ) -> None:
        nonlocal new_conflicts
        key = (
            message_id,
            field_path,
            sha256(str(existing)),
            sha256(str(candidate)),
        )
        if key in existing_conflict_keys:
            return
        existing_conflict_keys.add(key)
        conflicts.append(
            {
                "schema": "doudou.conflict.v2",
                "message_id": message_id,
                "field": field_path,
                "occurred_at": utc_now(),
                "existing_sha256": key[2],
                "incoming_sha256": key[3],
            }
        )
        new_conflicts += 1

    def merge_identity_value(
        existing: dict[str, Any], candidate: dict[str, Any]
    ) -> dict[str, Any]:
        merged = {**existing, **candidate}
        merged.update(merge_identity(existing, candidate))
        return merged

    def merge_identity_list(existing: list[Any], candidate: list[Any]) -> list[Any]:
        merged = [dict(item) if isinstance(item, dict) else item for item in existing]
        for item in candidate:
            if not isinstance(item, dict):
                if item not in merged:
                    merged.append(item)
                continue
            for index, current in enumerate(merged):
                if isinstance(current, dict) and identity_matches(current, item):
                    merged[index] = merge_identity_value(current, item)
                    break
            else:
                merged.append(dict(item))
        return merged

    def merge_value(
        existing: Any, candidate: Any, message_id: str, field_path: str
    ) -> Any:
        if existing in (None, "", [], {}) and candidate not in (None, "", [], {}):
            return candidate
        if field_path.endswith(".enrichment.updated_at"):
            return existing or candidate
        if (
            isinstance(existing, list)
            and isinstance(candidate, list)
            and field_path in IDENTITY_LIST_FIELDS
        ):
            return merge_identity_list(existing, candidate)
        if (
            isinstance(existing, dict)
            and isinstance(candidate, dict)
            and any(
                existing.get(key) or candidate.get(key)
                for key in ("uid", "sec_uid", "unique_id")
            )
        ):
            return merge_identity_value(existing, candidate)
        if isinstance(existing, dict) and isinstance(candidate, dict):
            merged = dict(existing)
            for key, value in candidate.items():
                child_path = f"{field_path}.{key}" if field_path else key
                merged[key] = merge_value(
                    merged.get(key), value, message_id, child_path
                )
            return merged
        if (
            candidate not in (None, "", [], {})
            and existing != candidate
            and field_path not in {"collected_at", "run_id"}
        ):
            if field_path.split(".", 1)[0] in AUTHORITATIVE_INCOMING_FIELDS:
                return candidate
            record_conflict(message_id, field_path, existing, candidate)
        return existing

    added = duplicates = 0
    for item in incoming:
        identity = str(item["message_id"])
        previous = by_id.get(identity)
        if previous is None:
            by_id[identity] = item
            added += 1
            continue
        duplicates += 1
        merged = dict(previous)
        for key, value in item.items():
            if key == "images":
                merged[key] = value
                continue
            merged[key] = merge_value(merged.get(key), value, identity, key)
        by_id[identity] = merged
    records = sorted(
        by_id.values(),
        key=lambda item: (
            int(item.get("create_time_ms") or 0),
            int(item["message_id"]),
        ),
    )
    atomic_write_jsonl(directory / "messages.jsonl", records)
    atomic_write_jsonl(directory / "conflicts.jsonl", conflicts)
    return {
        "directory": str(directory),
        "messages_file": str(directory / "messages.jsonl"),
        "total": len(records),
        "added": added,
        "duplicates": duplicates,
        "conflicts": new_conflicts,
        "conflicts_total": len(conflicts),
    }


def read_messages(
    data_root: Path, profile: str, conversation: dict[str, Any]
) -> list[dict[str, Any]]:
    directory = profile_paths(data_root, profile).conversations / conversation_folder(
        conversation
    )
    return read_jsonl(directory / "messages.jsonl", optional=True)
